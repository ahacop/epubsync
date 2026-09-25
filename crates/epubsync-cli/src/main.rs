//! The EpubSync command line. Every command except `init` loads the config
//! and opens the library, which takes the lock for the length of the run.

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use epubsync_core::config::{self, Config};
use epubsync_core::device::{Action, Device, ReadStatus, RowUpdate};
use epubsync_core::kobo::eject::Ejected;
use epubsync_core::kobo::{self, Kobo};
use epubsync_core::library::{Book, Field, HistoryRow, ImportOutcome, Library, ProgressRow};
use epubsync_core::metadata::{Author, Metadata, Series, format_series_number};
use epubsync_core::query::{Filter, Query, Sort, SortKey};
use epubsync_core::sort_name::sort_name;
use epubsync_core::sync::{self as core_sync, Gate};
use serde::Serialize;

#[derive(Parser)]
#[command(
    name = "epubsync",
    version = env!("EPUBSYNC_VERSION"),
    about = "Manage a KEPUB library and sync it to a Kobo"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the library folder and its database, and point the config at it
    Init { folder: PathBuf },
    /// Convert an EPUB to KEPUB and add it to the library. A folder imports every EPUB in it
    Import {
        path: PathBuf,
        /// Import even when a book with the same title and first author exists
        #[arg(long)]
        force: bool,
    },
    /// List the books with their id, metadata, and progress per device
    List {
        /// Show only the books with this text in the title, an author name, or the series name
        text: Option<String>,
        /// Show only the books with this text in the title
        #[arg(long)]
        title: Option<String>,
        /// Show only the books with this text in an author name
        #[arg(long)]
        author: Option<String>,
        /// Show only the books with this text in the series name
        #[arg(long)]
        series: Option<String>,
        /// Show only the books being read, by the progress row read last
        #[arg(long, group = "status")]
        reading: bool,
        /// Show only the finished books, by the progress row read last
        #[arg(long, group = "status")]
        finished: bool,
        /// Show only the books not started on any device
        #[arg(long, group = "status")]
        unread: bool,
        /// The order of the books. Several keys, as "author,title", break ties in turn. A book with no value for a key comes last
        #[arg(
            long,
            value_enum,
            value_delimiter = ',',
            default_value = "id",
            value_name = "KEY"
        )]
        sort: Vec<SortFlag>,
        /// Reverse the order
        #[arg(long)]
        reverse: bool,
        /// Print the books as a JSON array
        #[arg(long)]
        json: bool,
    },
    /// Print one book's whole record, its stats, its file path, and its progress per device
    Show {
        book: i64,
        /// Print the book as a JSON object
        #[arg(long)]
        json: bool,
    },
    /// Open a book's file in the system reader
    Open { book: i64 },
    /// Edit a book's metadata in $EDITOR, or one field per flag
    Edit {
        book: i64,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        publisher: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// An author as "Name" or "Name|Sort name". Repeat for several. Replaces the list
        #[arg(long = "author")]
        authors: Vec<String>,
        #[arg(long)]
        series: Option<String>,
        #[arg(long)]
        series_number: Option<f64>,
    },
    /// Take a book out of the library
    Remove {
        book: i64,
        /// Do not ask for confirmation
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Make the device folder match the library and read progress and words back
    Sync {
        /// Print the plan and change nothing
        #[arg(long)]
        dry_run: bool,
        /// The mounted device volume, instead of scanning the usual mount folders
        #[arg(long)]
        device: Option<PathBuf>,
        /// Write to the Kobo database on a firmware version the app has not been tested with
        #[arg(long)]
        allow_newer_firmware: bool,
        /// Run the deletes without asking
        #[arg(long, short = 'y')]
        yes: bool,
        /// Print the plan as a JSON object. Needs --dry-run
        #[arg(long, requires = "dry_run")]
        json: bool,
    },
    /// Unmount the Kobo and tell it the USB session is over
    Eject {
        /// The mounted device volume, instead of scanning the usual mount folders
        #[arg(long)]
        device: Option<PathBuf>,
    },
    /// List the words looked up on the device, newest first
    Words {
        #[arg(long)]
        book: Option<i64>,
        /// A device serial
        #[arg(long)]
        device: Option<String>,
        /// Print the words as a JSON array
        #[arg(long)]
        json: bool,
    },
}

/// The `--sort` values, one per core sort key.
#[derive(Debug, Clone, Copy, ValueEnum)]
enum SortFlag {
    Id,
    Title,
    Author,
    Series,
    Words,
    Ease,
    Progress,
    LastRead,
    Finished,
}

impl From<SortFlag> for SortKey {
    fn from(flag: SortFlag) -> SortKey {
        match flag {
            SortFlag::Id => SortKey::Id,
            SortFlag::Title => SortKey::Title,
            SortFlag::Author => SortKey::Author,
            SortFlag::Series => SortKey::Series,
            SortFlag::Words => SortKey::Words,
            SortFlag::Ease => SortKey::Ease,
            SortFlag::Progress => SortKey::Progress,
            SortFlag::LastRead => SortKey::LastRead,
            SortFlag::Finished => SortKey::Finished,
        }
    }
}

fn main() -> ExitCode {
    // Let a closed pipe end the process quietly, as in `list | head`,
    // instead of a panic on the next print.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let cli = Cli::parse();
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(command: Command) -> Result<()> {
    let config_path = config::path()?;
    if let Command::Init { folder } = &command {
        let lib = Library::init(folder)?;
        let config = Config {
            library: lib.folder.clone(),
        };
        config::save(&config_path, &config)?;
        println!("created the library at {}", lib.folder.display());
        println!("config written to {}", config_path.display());
        return Ok(());
    }
    let config = config::load(&config_path)?;
    match command {
        Command::Init { .. } => unreachable!(),
        Command::Import { path, force } => import(&config, &path, force),
        Command::List {
            text,
            title,
            author,
            series,
            reading,
            finished,
            unread,
            sort,
            reverse,
            json,
        } => {
            let status = if reading {
                Some(ReadStatus::Reading)
            } else if finished {
                Some(ReadStatus::Finished)
            } else if unread {
                Some(ReadStatus::Unread)
            } else {
                None
            };
            let query = Query {
                filter: Filter {
                    text: text.unwrap_or_default(),
                    title: title.unwrap_or_default(),
                    author: author.unwrap_or_default(),
                    series: series.unwrap_or_default(),
                    status,
                },
                sort: Sort {
                    keys: sort.into_iter().map(SortKey::from).collect(),
                    descending: reverse,
                },
            };
            list(&config, &query, json)
        }
        Command::Show { book, json } => show(&config, book, json),
        Command::Open { book } => open(&config, book),
        Command::Edit {
            book,
            title,
            publisher,
            description,
            authors,
            series,
            series_number,
        } => {
            let flags = EditFlags {
                title,
                publisher,
                description,
                authors,
                series,
                series_number,
            };
            edit(&config, book, flags)
        }
        Command::Remove { book, yes } => remove(&config, book, yes),
        Command::Sync {
            dry_run,
            device,
            allow_newer_firmware,
            yes,
            json,
        } => sync(
            &config,
            SyncFlags {
                dry_run,
                device,
                allow_newer_firmware,
                yes,
                json,
            },
        ),
        Command::Words { book, device, json } => words(&config, book, device.as_deref(), json),
        Command::Eject { device } => {
            let kobo = find_kobo(device.as_deref())?;
            eject(&kobo)
        }
    }
}

fn import(config: &Config, path: &Path, force: bool) -> Result<()> {
    let mut lib = Library::open(config)?;
    let files = if path.is_dir() {
        let mut files: Vec<PathBuf> = std::fs::read_dir(path)
            .with_context(|| format!("read {}", path.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.to_string_lossy().ends_with(".epub"))
            .collect();
        files.sort();
        files
    } else {
        vec![path.to_path_buf()]
    };
    if files.is_empty() {
        bail!("no .epub files in {}", path.display());
    }
    let mut failed = 0;
    for file in &files {
        match lib.import(file, force) {
            Ok(ImportOutcome::Imported { id, made_sort }) => {
                let book = lib.get(id)?;
                println!("{id:>5}  {}  {}", book.metadata.title, file.display());
                for author in made_sort {
                    println!("       made sort name for {}: {}", author.name, author.sort);
                }
            }
            Ok(ImportOutcome::Exists { id }) => {
                println!(
                    "{id:>5}  already in the library, skipped  {}",
                    file.display()
                );
            }
            Err(e) => {
                failed += 1;
                println!("error  {}: {e:#}", file.display());
            }
        }
    }
    if failed > 0 {
        bail!("{failed} of {} files failed", files.len());
    }
    Ok(())
}

/// One book as `--json` prints it: the flat record, the file path, and
/// the progress per device. `list` prints an array of these and `show`
/// prints one, with the book's progress history too.
#[derive(Serialize)]
struct BookJson<'a> {
    #[serde(flatten)]
    book: &'a Book,
    file: PathBuf,
    progress: &'a [ProgressRow],
    #[serde(skip_serializing_if = "Option::is_none")]
    history: Option<&'a [HistoryRow]>,
}

/// Prints a value as indented JSON with a newline at the end.
fn print_json<T: Serialize>(value: &T) -> Result<()> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer_pretty(&mut out, value)?;
    out.write_all(b"\n")?;
    Ok(())
}

fn list(config: &Config, query: &Query, json: bool) -> Result<()> {
    let lib = Library::open(config)?;
    let books = lib.list()?;
    let progress = lib.progress()?;
    let shown = query.select(&books, &progress);
    if json {
        let rows: Vec<BookJson> = shown
            .iter()
            .map(|book| BookJson {
                book,
                file: lib.book_path(book.id),
                progress: progress.get(&book.id).map_or(&[], Vec::as_slice),
                history: None,
            })
            .collect();
        return print_json(&rows);
    }
    for book in shown {
        let mut line = book_line(book);
        for p in progress.get(&book.id).into_iter().flatten() {
            line.push_str(&format!("  {}", progress_cell(p)));
        }
        println!("{line}");
    }
    Ok(())
}

/// One label and value per line, then the description as Markdown after a
/// blank line. A field the book does not have gets no line. The Device
/// lines hold the progress per device, and the History lines hold every
/// change a sync saw, oldest first.
/// Hands the book's file to the system's opener: `xdg-open` on Linux,
/// `open` on macOS. The library lock is let go first, so the viewer or
/// another command can run while the reader is open.
fn open(config: &Config, id: i64) -> Result<()> {
    let path = {
        let lib = Library::open(config)?;
        lib.get(id)?;
        lib.book_path(id)
    };
    opener::open(&path).with_context(|| format!("open {}", path.display()))
}

fn show(config: &Config, id: i64, json: bool) -> Result<()> {
    let lib = Library::open(config)?;
    let book = lib.get(id)?;
    let progress = lib.book_progress(id)?;
    let history = lib.book_history(id)?;
    if json {
        return print_json(&BookJson {
            book: &book,
            file: lib.book_path(id),
            progress: &progress,
            history: Some(&history),
        });
    }
    let m = &book.metadata;

    let mut lines = vec![("Id", book.id.to_string()), ("Title", m.title.clone())];
    for author in &m.authors {
        lines.push(("Author", format!("{} (sort: {})", author.name, author.sort)));
    }
    lines.extend(book.fields().into_iter().map(|f| match f {
        Field::Publisher(p) => ("Publisher", p.to_string()),
        // A series can have no number: a file with a series name and no
        // index, or an edit that sets `--series` alone.
        Field::Series(s) => match s.number {
            Some(n) => (
                "Series",
                format!("{}, book {}", s.name, format_series_number(n)),
            ),
            None => ("Series", s.name.clone()),
        },
        Field::WordCount(w) => ("Words", w.to_string()),
        Field::ReadingEase(e) => ("Ease", format!("{e:.0}")),
    }));
    lines.push(("Revision", book.revision.to_string()));
    lines.push(("File", lib.book_path(id).display().to_string()));
    if progress.is_empty() {
        lines.push(("Device", "not yet sent to a device".to_string()));
    }
    for p in &progress {
        lines.push(("Device", progress_cell(p)));
    }
    for h in &history {
        let cell = format!(
            "{}  {}: {}% {} {}",
            h.seen_day(),
            h.device_serial,
            h.percent,
            status_word(h.status),
            h.day().unwrap_or("")
        );
        lines.push(("History", cell.trim_end().to_string()));
    }

    for (label, value) in lines {
        println!("{label:<11}{value}");
    }
    if let Some(d) = &m.description {
        // htmd fails only when its writer fails, which a String does not.
        let text = htmd::convert(d).unwrap_or_else(|_| d.clone());
        println!("\n{}", text.trim());
    }
    Ok(())
}

/// One device's progress: the serial, the percent, the status, and the
/// day last read. A finished book shows the day it was finished in
/// place of the day last read, and a book finished before and opened
/// again shows both. The reading time comes last, when the device
/// counted any.
fn progress_cell(p: &ProgressRow) -> String {
    let mut cell = format!("{}: {}% ", p.device_serial, p.percent);
    match (p.status, p.finished_day()) {
        (ReadStatus::Finished, Some(day)) => cell.push_str(&format!("finished {day}")),
        (status, finished) => {
            cell.push_str(status_word(status));
            if let Some(day) = p.day() {
                cell.push_str(&format!(" {day}"));
            }
            if let Some(day) = finished {
                cell.push_str(&format!(", finished {day}"));
            }
        }
    }
    if let Some(seconds) = p.time_spent.filter(|s| *s > 0) {
        cell.push_str(&format!(", {}", duration(seconds)));
    }
    cell.trim_end().to_string()
}

fn status_word(status: ReadStatus) -> &'static str {
    match status {
        ReadStatus::Unread => "unread",
        ReadStatus::Reading => "reading",
        ReadStatus::Finished => "finished",
    }
}

/// A reading time in seconds as "3 h 20 min", "20 min", or "less than a
/// minute".
fn duration(seconds: i64) -> String {
    let minutes = seconds / 60;
    match (minutes / 60, minutes % 60) {
        (0, 0) => "less than a minute".to_string(),
        (0, m) => format!("{m} min"),
        (h, m) => format!("{h} h {m} min"),
    }
}

fn words(config: &Config, book: Option<i64>, device: Option<&str>, json: bool) -> Result<()> {
    let lib = Library::open(config)?;
    let words = lib.words(book, device)?;
    if json {
        return print_json(&words);
    }
    for w in words {
        println!(
            "{}  {:<24}  {:>5}  {}  ({})",
            w.looked_up_at, w.word, w.book_id, w.book_title, w.device_serial
        );
    }
    Ok(())
}

fn book_line(book: &Book) -> String {
    let m = &book.metadata;
    let authors: Vec<&str> = m.authors.iter().map(|a| a.name.as_str()).collect();
    let mut line = format!("{:>5}  {}  by {}", book.id, m.title, authors.join(" & "));
    if let Some(s) = &m.series {
        line.push_str(&format!("  [{}", s.name));
        if let Some(n) = s.number {
            line.push_str(&format!(" #{}", format_series_number(n)));
        }
        line.push(']');
    }
    line
}

struct EditFlags {
    title: Option<String>,
    publisher: Option<String>,
    description: Option<String>,
    authors: Vec<String>,
    series: Option<String>,
    series_number: Option<f64>,
}

impl EditFlags {
    fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.publisher.is_none()
            && self.description.is_none()
            && self.authors.is_empty()
            && self.series.is_none()
            && self.series_number.is_none()
    }
}

fn edit(config: &Config, id: i64, flags: EditFlags) -> Result<()> {
    let mut lib = Library::open(config)?;
    let book = lib.get(id)?;
    let record = if flags.is_empty() {
        edit_in_editor(&book.metadata)?
    } else {
        apply_flags(book.metadata, flags)?
    };
    lib.edit(id, &record)?;
    println!("{}", book_line(&lib.get(id)?));
    Ok(())
}

fn apply_flags(mut record: Metadata, flags: EditFlags) -> Result<Metadata> {
    if let Some(t) = flags.title {
        record.title = t;
    }
    if let Some(p) = flags.publisher {
        record.publisher = Some(p).filter(|p| !p.is_empty());
    }
    if let Some(d) = flags.description {
        record.description = Some(d).filter(|d| !d.is_empty());
    }
    if !flags.authors.is_empty() {
        record.authors = flags
            .authors
            .iter()
            .map(|a| match a.split_once('|') {
                Some((name, sort)) => Author {
                    name: name.trim().to_string(),
                    sort: sort.trim().to_string(),
                },
                None => Author {
                    name: a.trim().to_string(),
                    sort: sort_name(a),
                },
            })
            .collect();
    }
    if let Some(name) = flags.series {
        if name.is_empty() {
            record.series = None;
        } else {
            let number = flags
                .series_number
                .or(record.series.as_ref().and_then(|s| s.number));
            record.series = Some(Series { name, number });
        }
    } else if let Some(n) = flags.series_number {
        match &mut record.series {
            Some(s) => s.number = Some(n),
            None => bail!("the book has no series. Pass --series with --series-number"),
        }
    }
    if record.title.trim().is_empty() {
        bail!("the title is empty");
    }
    Ok(record)
}

const EDIT_HEADER: &str = "# Edit the fields and save. An empty file cancels.
# authors is a list of tables with name and sort. series has name and number.
# Remove the series, publisher, or description block to clear the field.
# The description is written into the file as it is, HTML included.

";

fn edit_in_editor(record: &Metadata) -> Result<Metadata> {
    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|e| !e.trim().is_empty())
        .ok_or_else(|| anyhow!("set $EDITOR, or pass a flag such as --title"))?;
    let mut file = tempfile::Builder::new()
        .prefix("epubsync-")
        .suffix(".toml")
        .tempfile()?;
    let text = format!("{EDIT_HEADER}{}", toml::to_string_pretty(record)?);
    file.write_all(text.as_bytes())?;
    file.flush()?;

    let mut parts = editor.split_whitespace();
    let program = parts.next().unwrap();
    let status = std::process::Command::new(program)
        .args(parts)
        .arg(file.path())
        .status()
        .with_context(|| format!("run {editor}"))?;
    if !status.success() {
        bail!("{editor} exited with {status}");
    }
    let edited = std::fs::read_to_string(file.path())?;
    let body: String = edited
        .lines()
        .filter(|l| !l.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    if body.trim().is_empty() {
        bail!("empty file, nothing changed");
    }
    let record: Metadata = toml::from_str(&body).context("parse the edited TOML")?;
    if record.title.trim().is_empty() {
        bail!("the title is empty");
    }
    Ok(record)
}

fn remove(config: &Config, id: i64, yes: bool) -> Result<()> {
    let mut lib = Library::open(config)?;
    let book = lib.get(id)?;
    if !yes {
        if !std::io::stdin().is_terminal() {
            bail!("pass --yes to remove without a prompt");
        }
        print!("remove {} \"{}\"? [y/N] ", book.id, book.metadata.title);
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("kept");
            return Ok(());
        }
    }
    lib.remove(id)?;
    println!("removed {} \"{}\"", book.id, book.metadata.title);
    Ok(())
}

struct SyncFlags {
    dry_run: bool,
    device: Option<PathBuf>,
    allow_newer_firmware: bool,
    yes: bool,
    /// Print the plan as JSON. Clap makes it require `dry_run`.
    json: bool,
}

/// The plan as `sync --dry-run --json` prints it.
#[derive(Serialize)]
struct PlanJson<'a> {
    device: DeviceJson<'a>,
    /// Why the device refuses row writes, or null when it accepts them.
    write_gate: Option<&'a str>,
    /// The actions sync would run.
    actions: Vec<ActionJson<'a>>,
    /// The replacements sync holds back while the write gate is closed.
    skipped: Vec<ActionJson<'a>>,
}

#[derive(Serialize)]
struct DeviceJson<'a> {
    serial: &'a str,
    root: &'a Path,
    db_version: Option<i64>,
}

/// One action with its book's title. A delete has no title: the book is
/// no longer in the library.
#[derive(Serialize)]
struct ActionJson<'a> {
    #[serde(flatten)]
    action: &'a Action,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
}

fn action_json<'a>(lib: &Library, action: &'a Action) -> Result<ActionJson<'a>> {
    let title = match action {
        Action::Delete { .. } => None,
        _ => Some(lib.get(action.id())?.metadata.title),
    };
    Ok(ActionJson { action, title })
}

fn plan_json<'a>(lib: &Library, kobo: &'a Kobo, gate: &'a Gate) -> Result<PlanJson<'a>> {
    let (write_gate, actions, skipped) = match gate {
        Gate::Open(actions) => (None, actions, &[][..]),
        Gate::Closed {
            kept,
            skipped,
            reason,
        } => (Some(reason.as_str()), kept, skipped.as_slice()),
    };
    Ok(PlanJson {
        device: DeviceJson {
            serial: kobo.serial(),
            root: &kobo.root,
            db_version: kobo.db_version(),
        },
        write_gate,
        actions: actions
            .iter()
            .map(|a| action_json(lib, a))
            .collect::<Result<_>>()?,
        skipped: skipped
            .iter()
            .map(|a| action_json(lib, a))
            .collect::<Result<_>>()?,
    })
}

fn sync(config: &Config, flags: SyncFlags) -> Result<()> {
    let mut lib = Library::open(config)?;
    let mut kobo = find_kobo(flags.device.as_deref())?;
    if !flags.json {
        println!("Kobo {} at {}", kobo.serial(), kobo.root.display());
    }
    kobo.open_db(flags.allow_newer_firmware)?;
    if let Some(v) = kobo.db_version()
        && !flags.json
    {
        println!("Kobo database version {v}");
    }

    let mut gate = core_sync::gate(core_sync::plan(&lib, &kobo)?, &kobo);
    if flags.json {
        return print_json(&plan_json(&lib, &kobo, &gate)?);
    }
    match &gate {
        Gate::Open(actions) => {
            if actions.is_empty() {
                println!("nothing to do");
            }
            for action in actions {
                println!("{}", action_line(&lib, action)?);
            }
        }
        Gate::Closed {
            kept,
            skipped,
            reason,
        } => {
            println!("{reason}");
            for action in kept {
                println!("{}", action_line(&lib, action)?);
            }
            for action in skipped {
                println!("skipped: {}", action_line(&lib, action)?);
            }
        }
    }
    let (Gate::Open(actions) | Gate::Closed { kept: actions, .. }) = &mut gate;
    if flags.dry_run {
        return Ok(());
    }

    let deletes = actions
        .iter()
        .filter(|a| matches!(a, Action::Delete { .. }))
        .count();
    if deletes > 0 && !flags.yes {
        if !std::io::stdin().is_terminal() {
            bail!("pass --yes to run the deletes without a prompt");
        }
        print!("delete {deletes} file(s) from the device? [y/N] ");
        std::io::stdout().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim(), "y" | "Y" | "yes") {
            println!("deletes skipped");
            actions.retain(|a| !matches!(a, Action::Delete { .. }));
        }
    }

    core_sync::apply(&mut lib, &mut kobo, actions, |a| {
        let verb = match a {
            Action::Send { .. } => "sending",
            Action::Replace { .. } => "replacing",
            Action::SendAgain { .. } => "sending again",
            Action::Delete { .. } => "deleting",
        };
        println!("{verb} {}", a.id());
    })?;
    let removed = kobo.remove_dot_underscore_files()?;
    if removed > 0 {
        println!("deleted {removed} macOS ._ file(s)");
    }

    if let Gate::Open(actions) = &gate {
        let sent_now: Vec<i64> = actions
            .iter()
            .filter(|a| !matches!(a, Action::Delete { .. }))
            .map(Action::id)
            .collect();
        for (id, outcome) in core_sync::update_rows(&lib, &mut kobo)? {
            match outcome {
                RowUpdate::Updated => println!("updated the Kobo row for {id}"),
                RowUpdate::NoRow if sent_now.contains(&id) => {
                    println!("series appears on the next sync for {id}")
                }
                RowUpdate::NoRow | RowUpdate::Unchanged => {}
            }
        }
    }

    let back = core_sync::read_back(&mut lib, &mut kobo)?;
    if !back.progress.is_empty() {
        println!(
            "read progress for {} book(s), {} changed",
            back.progress.len(),
            back.changed
        );
    }
    if !back.words.is_empty() {
        println!("{} new word(s)", back.words.len());
    }
    kobo.finish()?;
    println!("run `epubsync eject` before you unplug the device");
    Ok(())
}

fn find_kobo(device: Option<&Path>) -> Result<Kobo> {
    match device {
        Some(path) => Kobo::at(path),
        None => kobo::detect_one(&kobo::default_roots())
            .map_err(|e| anyhow!("{e}. Plug in one Kobo, or pass --device <path>")),
    }
}

fn eject(kobo: &Kobo) -> Result<()> {
    match kobo.eject()? {
        Ejected::Yes => println!("ejected, unplug the device"),
        Ejected::NotAVolume => println!(
            "{} is a folder, not a mounted volume; nothing to eject",
            kobo.root.display()
        ),
    }
    Ok(())
}

fn action_line(lib: &Library, action: &Action) -> Result<String> {
    let title = |id: i64| -> String { lib.get(id).map(|b| b.metadata.title).unwrap_or_default() };
    Ok(match action {
        Action::Send { id, .. } => format!("send        {id:>5}  {}", title(*id)),
        Action::Replace { id, .. } => format!("replace     {id:>5}  {}", title(*id)),
        Action::SendAgain { id, .. } => {
            format!("send again  {id:>5}  {}  (deleted on device)", title(*id))
        }
        Action::Delete { id } => format!("delete      {id:>5}  (no longer in the library)"),
    })
}
