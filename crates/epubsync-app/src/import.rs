//! Import from the viewer: the Import button and a drop of files onto the
//! window. Each file goes through `Library::import` on a background task,
//! because kepubify takes seconds per book and the measurement takes
//! more. The task takes the `Library` value with it and hands it back
//! with the outcome, so the state holds no library while a file imports.
//!
//! A strip under the toolbar shows the file in flight, a progress bar, a
//! Cancel button, and three tabs: Added, Skipped, Failed, each with its
//! count. While the strip is shown, the main pane draws the rows of the
//! tab in view in place of the library query: the added books, the
//! library books the skipped files matched, or the failed files with
//! their errors. The strip lists nothing itself, so its height is the
//! same for one file and for a thousand.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use epubsync_core::library::{ImportOutcome, Library};
use iced::widget::{button, column, container, progress_bar, row, space, text};
use iced::{Center, Element, Fill, Task, padding};

use crate::handoff::Handoff;
use crate::table::file_name;
use crate::theme::{BODY, MONO, SANS_MEDIUM, SANS_SEMIBOLD};
use crate::{Message, Open, format, table, theme};

/// What one file's import came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// The book is in the library as book `id`.
    Added { id: i64 },
    /// A book with the same title and first author is already in the
    /// library, as book `id`.
    Skipped { id: i64 },
    /// The import failed. `error` is the whole error chain.
    Failed { file: PathBuf, error: String },
}

/// One of the strip's tabs. Each shows the lines of one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Added,
    Skipped,
    Failed,
}

const TABS: [Tab; 3] = [Tab::Added, Tab::Skipped, Tab::Failed];

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::Added => "Added",
            Tab::Skipped => "Skipped",
            Tab::Failed => "Failed",
        }
    }

    /// Whether a line belongs to this tab.
    fn holds(self, line: &Line) -> bool {
        matches!(
            (self, line),
            (Tab::Added, Line::Added { .. })
                | (Tab::Skipped, Line::Skipped { .. })
                | (Tab::Failed, Line::Failed { .. })
        )
    }
}

/// An import under way or done: the files still to go, the one in
/// flight, a line per file done, the tab in view, and when it started
/// and ended.
#[derive(Debug, Clone)]
pub struct Import {
    queue: VecDeque<PathBuf>,
    current: Option<PathBuf>,
    lines: Vec<Line>,
    tab: Tab,
    started: Instant,
    /// When the last file finished. None while files are to go.
    ended: Option<Instant>,
    /// Whether Cancel cut the queue short.
    cancelled: bool,
}

impl Import {
    /// An empty import, started now, on the Added tab.
    pub fn new() -> Import {
        Import {
            queue: VecDeque::new(),
            current: None,
            lines: Vec::new(),
            tab: Tab::Added,
            started: Instant::now(),
            ended: None,
            cancelled: false,
        }
    }

    /// Whether a file is on the task now.
    pub fn running(&self) -> bool {
        self.current.is_some()
    }

    /// Puts a tab in view.
    pub fn show(&mut self, tab: Tab) {
        self.tab = tab;
    }

    /// Queues a path. A folder gives its `.epub` files one level deep,
    /// in name order, the same as the CLI, and a folder with none gets a
    /// failed line. Any other path is queued as one file.
    pub fn add(&mut self, path: &Path) {
        if !path.is_dir() {
            self.queue.push_back(path.to_path_buf());
            return;
        }
        match epubs_in(path) {
            Ok(files) if files.is_empty() => self.lines.push(Line::Failed {
                file: path.to_path_buf(),
                error: "no .epub files in the folder".to_string(),
            }),
            Ok(files) => self.queue.extend(files),
            Err(e) => self.lines.push(Line::Failed {
                file: path.to_path_buf(),
                error: format!("{e:#}"),
            }),
        }
    }

    /// Starts the next queued file when none is in flight and the state
    /// holds the library. The task takes the library and gives it back
    /// with the file's line in `Message::Imported`. With no file left to
    /// start, the import ends here.
    pub fn start(&mut self, library: &mut Option<Library>) -> Task<Message> {
        if self.current.is_some() || library.is_none() {
            return Task::none();
        }
        let Some(file) = self.queue.pop_front() else {
            self.ended.get_or_insert_with(Instant::now);
            return Task::none();
        };
        let mut lib = library.take().expect("the state holds the library");
        self.current = Some(file.clone());
        Task::perform(
            async move {
                let line = import_one(&mut lib, file);
                (Handoff::new(lib), line)
            },
            |(handoff, line)| Message::Imported(handoff, line),
        )
    }

    /// Records the line of the file that was in flight.
    pub fn finish(&mut self, line: Line) {
        self.current = None;
        self.lines.push(line);
    }

    /// Drops the queued files. The file in flight finishes, because
    /// `Library::import` is one call, and then the import ends.
    pub fn cancel(&mut self) {
        self.queue.clear();
        self.cancelled = true;
    }

    /// How many lines a tab holds.
    fn count(&self, tab: Tab) -> usize {
        self.lines.iter().filter(|l| tab.holds(l)).count()
    }

    /// The book ids of the Added or the Skipped tab, in import order.
    /// The Failed tab has no books.
    fn ids(&self, tab: Tab) -> impl Iterator<Item = i64> + '_ {
        self.lines.iter().filter_map(move |line| match line {
            Line::Added { id } | Line::Skipped { id } if tab.holds(line) => Some(*id),
            _ => None,
        })
    }

    /// The failed files and their errors, in import order.
    fn failed(&self) -> Vec<(&Path, &str)> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Failed { file, error } => Some((file.as_path(), error.as_str())),
                _ => None,
            })
            .collect()
    }

    /// How long the import took, or has taken so far.
    fn elapsed(&self) -> Duration {
        self.ended.unwrap_or_else(Instant::now) - self.started
    }
}

fn import_one(lib: &mut Library, file: PathBuf) -> Line {
    match lib.import(&file, false) {
        Ok(ImportOutcome::Imported { id, .. }) => Line::Added { id },
        Ok(ImportOutcome::Exists { id }) => Line::Skipped { id },
        Err(e) => Line::Failed {
            file,
            error: format!("{e:#}"),
        },
    }
}

/// The `.epub` files in a folder, one level deep, in name order.
fn epubs_in(folder: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.to_string_lossy().ends_with(".epub"))
        .collect();
    files.sort();
    Ok(files)
}

/// Opens the system file picker on EPUB files. The picked paths come
/// back in `Message::Picked`; a cancel gives none.
pub fn pick() -> Task<Message> {
    let dialog = rfd::AsyncFileDialog::new()
        .set_title("Import EPUB files")
        .add_filter("EPUB", &["epub", "kepub"]);
    Task::perform(dialog.pick_files(), |picked| {
        let paths = picked
            .unwrap_or_default()
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect();
        Message::Picked(paths)
    })
}

/// "Imported 12 files in 1 min 14 s", or "Cancelled after 4 files in
/// 20 s" when Cancel cut the queue short. The count is every file that
/// went through, whatever it came to; the tabs break it down.
fn summary(import: &Import) -> String {
    let files = crate::count(import.lines.len(), "file");
    let took = format::elapsed(import.elapsed());
    if import.cancelled {
        format!("Cancelled after {files} in {took}")
    } else {
        format!("Imported {files} in {took}")
    }
}

/// The strip under the toolbar. While a file is in flight the first line
/// reads "Importing 3 of 12 · pride.epub" with a Cancel button, and a
/// progress bar sits under it. When the queue is done the first line
/// reads the summary with a × that clears the strip. The tabs come last
/// in both cases.
pub fn view(import: &Import) -> Element<'_, Message> {
    let done = import.lines.len();
    let mut body = column![].spacing(6);
    match &import.current {
        Some(file) => {
            let n = done + 1;
            let total = n + import.queue.len();
            let word = if import.cancelled {
                "Cancelling".to_string()
            } else {
                format!("Importing {n} of {total}")
            };
            let cancel = button(text("Cancel").size(13))
                .on_press_maybe((!import.cancelled).then_some(Message::CancelImport))
                .padding([5, 10])
                .style(theme::action);
            let head = row![
                text(word).size(BODY).font(SANS_MEDIUM),
                text(file_name(file))
                    .size(BODY)
                    .style(theme::text_color(|c| c.muted)),
                space().width(Fill),
                cancel,
            ]
            .spacing(10)
            .align_y(Center);
            let bar = progress_bar(0.0..=total as f32, done as f32)
                .girth(4)
                .style(theme::import_bar);
            body = body
                .push(head)
                .push(container(bar).padding(padding::top(2).right(6)));
        }
        None => {
            let close = button(container(text("×").size(15)).center(22))
                .on_press(Message::ClearImport)
                .padding(0)
                .style(theme::close);
            let head = row![
                text(summary(import)).size(BODY).font(SANS_MEDIUM),
                space().width(Fill),
                close,
            ]
            .align_y(Center);
            body = body.push(head);
        }
    }
    let tabs = row(TABS.map(|t| tab(import, t)))
        .spacing(18)
        .align_y(Center)
        .padding(padding::top(2));
    body = body.push(tabs);
    column![
        container(body)
            .width(Fill)
            .padding(padding::all(10).left(14).right(8))
            .style(theme::ground(|c| c.window)),
        theme::hline(),
    ]
    .into()
}

/// One tab: the name, its count in the monospace face, and on the tab in
/// view a 2 px `accent` mark along the bottom edge, the same mark the
/// sorted column header wears.
fn tab<'a>(import: &Import, tab: Tab) -> Element<'a, Message> {
    let active = import.tab == tab;
    let count = text(import.count(tab).to_string())
        .size(12)
        .font(MONO)
        .style(if active {
            theme::text_color(|c| c.ink_2)
        } else {
            theme::text_color(|c| c.faint)
        });
    let label = row![text(tab.name()).size(13).font(SANS_SEMIBOLD), count]
        .spacing(6)
        .align_y(Center);
    let mark = container(space()).width(Fill).height(2);
    let mark = if active {
        mark.style(theme::ground(|c| c.accent))
    } else {
        mark
    };
    button(column![
        container(label).padding(padding::vertical(2)),
        mark
    ])
    .on_press(Message::ImportTab(tab))
    .padding(0)
    .style(theme::tab(active))
    .into()
}

/// The main pane while the strip is shown: the rows of the tab in view,
/// and their count for the toolbar. The Added and the Skipped tabs put
/// their library books through the query, so the headers sort them and
/// the filter field narrows them. The Failed tab lists the files.
pub fn pane<'a>(open: &'a Open, import: &'a Import) -> (Element<'a, Message>, usize) {
    match import.tab {
        Tab::Failed => {
            let rows = import.failed();
            let n = rows.len();
            (table::failed_view(open, rows), n)
        }
        tab => {
            // The books are in id order, so a binary search finds each.
            let books = import.ids(tab).filter_map(|id| {
                let i = open.books.binary_search_by_key(&id, |b| b.id).ok()?;
                Some(&open.books[i])
            });
            let rows = open.query.select(books, &open.progress);
            let n = rows.len();
            (table::view(open, rows), n)
        }
    }
}

/// The strip while files hover over the window with no import in view.
pub fn drop_hint<'a>() -> Element<'a, Message> {
    column![
        container(
            text("Drop EPUB files to import them")
                .size(BODY)
                .font(SANS_MEDIUM)
                .style(theme::text_color(|c| c.accent))
        )
        .width(Fill)
        .padding(padding::all(10).left(14))
        .style(theme::ground(|c| c.accent_tint)),
        theme::hline(),
    ]
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(kind: u8) -> Line {
        match kind {
            0 => Line::Added { id: 7 },
            1 => Line::Skipped { id: 4 },
            _ => Line::Failed {
                file: PathBuf::from("b.epub"),
                error: "bad".to_string(),
            },
        }
    }

    fn with_lines(kinds: &[u8]) -> Import {
        let mut import = Import::new();
        import.lines = kinds.iter().map(|&k| line(k)).collect();
        import
    }

    #[test]
    fn summary_counts_every_file_and_says_when_cancelled() {
        let mut import = with_lines(&[]);
        import.ended = Some(import.started + Duration::from_secs(74));
        assert_eq!(summary(&import), "Imported 0 files in 1 min 14 s");
        let mut import = with_lines(&[0]);
        import.ended = Some(import.started + Duration::from_secs(3));
        assert_eq!(summary(&import), "Imported 1 file in 3 s");
        let mut import = with_lines(&[0, 0, 1, 2, 2]);
        import.ended = Some(import.started + Duration::from_secs(20));
        import.cancelled = true;
        assert_eq!(summary(&import), "Cancelled after 5 files in 20 s");
    }

    #[test]
    fn each_tab_holds_its_own_lines() {
        let import = with_lines(&[0, 0, 1, 2, 2, 2]);
        assert_eq!(import.count(Tab::Added), 2);
        assert_eq!(import.count(Tab::Skipped), 1);
        assert_eq!(import.count(Tab::Failed), 3);
        assert_eq!(import.ids(Tab::Added).collect::<Vec<_>>(), [7, 7]);
        assert_eq!(import.ids(Tab::Skipped).collect::<Vec<_>>(), [4]);
        assert_eq!(import.ids(Tab::Failed).count(), 0);
        assert_eq!(import.failed().len(), 3);
        assert_eq!(import.failed()[0], (Path::new("b.epub"), "bad"));
    }

    #[test]
    fn cancel_drops_the_queue_and_keeps_the_file_in_flight() {
        let mut import = Import::new();
        import.queue.extend(["a.epub", "b.epub"].map(PathBuf::from));
        import.current = Some(PathBuf::from("c.epub"));
        import.cancel();
        assert!(import.queue.is_empty());
        assert!(import.running());
        assert!(import.cancelled);
        assert!(import.ended.is_none());
    }

    #[test]
    fn start_with_nothing_to_go_ends_the_import() {
        let dir = tempfile::tempdir().unwrap();
        let mut library = Some(Library::init(&dir.path().join("library")).unwrap());
        let mut import = Import::new();
        assert!(import.ended.is_none());
        let _ = import.start(&mut library);
        assert!(import.ended.is_some());
        assert!(library.is_some());
    }

    #[test]
    fn add_queues_a_file_and_expands_a_folder() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.epub", "a.epub", "notes.txt"] {
            std::fs::write(dir.path().join(name), b"").unwrap();
        }
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/c.epub"), b"").unwrap();

        let mut import = Import::new();
        import.add(&dir.path().join("notes.txt"));
        import.add(dir.path());
        let queued: Vec<PathBuf> = import.queue.iter().cloned().collect();
        assert_eq!(
            queued,
            [
                dir.path().join("notes.txt"),
                dir.path().join("a.epub"),
                dir.path().join("b.epub"),
            ]
        );
        assert!(import.lines.is_empty());
    }

    #[test]
    fn add_of_an_empty_folder_is_a_failed_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut import = Import::new();
        import.add(dir.path());
        assert!(import.queue.is_empty());
        assert_eq!(
            import.lines,
            [Line::Failed {
                file: dir.path().to_path_buf(),
                error: "no .epub files in the folder".to_string(),
            }]
        );
    }
}

#[cfg(test)]
mod library_tests {
    use super::*;
    use epubsync_epub::fixtures;

    /// The three lines one file can come to, against a real library:
    /// added on the first import, skipped on the second because the
    /// title and first author match, and failed for a file that is
    /// not an EPUB.
    #[test]
    fn import_one_gives_each_line() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::init(&dir.path().join("library")).unwrap();
        let epub = fixtures::write_epub(dir.path(), "lhod.epub", fixtures::EPUB2_OPF);
        let junk = dir.path().join("junk.epub");
        std::fs::write(&junk, b"not a zip").unwrap();

        assert_eq!(import_one(&mut lib, epub.clone()), Line::Added { id: 1 });
        assert_eq!(import_one(&mut lib, epub.clone()), Line::Skipped { id: 1 });
        let Line::Failed { file, error } = import_one(&mut lib, junk.clone()) else {
            panic!("junk imported");
        };
        assert_eq!(file, junk);
        assert!(!error.is_empty());
        assert_eq!(lib.list().unwrap().len(), 1);
    }
}
