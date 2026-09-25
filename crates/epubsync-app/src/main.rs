//! The library viewer: a window with a table of books and, when a book is
//! selected, its cover and its details in a sidebar on the right. A Words
//! tab swaps the table for the list of words looked up on a device. A
//! Reload button reads the library again, and an Import button or a drop
//! of files onto the window adds books, with a strip under the toolbar
//! that shows the progress and gives the table the added books. A Remove…
//! button in the sidebar removes the selected book after a dialog, and an
//! Open button there opens the book in the system reader. A click on the
//! cover there opens it at full size over the window. A link in a
//! description opens in the browser. An Edit button in the sidebar turns
//! the body into a form that writes the book's fields back to the
//! library. A Sync tab looks for the Kobo and shows the plan, which the
//! Sync button there runs, and an Eject button there ejects the Kobo.

mod description;
mod detail;
mod dictionary;
mod edit;
mod format;
mod full_cover;
mod handoff;
mod import;
mod remove;
mod sync;
mod table;
mod theme;
mod words;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use epubsync_core::config;
use epubsync_core::cover::Cover;
use epubsync_core::device::ReadStatus;
use epubsync_core::kobo::Kobo;
use epubsync_core::library::{Book, FileCover, Library, ProgressRow, WordRow, book_file_name};
use epubsync_core::query::{self, Query, Sort, SortKey};
use iced::event::{self, Event, Status};
use iced::keyboard::{self, key};
use iced::widget::{button, column, container, image, markdown, row, space, text, text_input};
use iced::{Center, Element, Fill, Subscription, Task, padding, window};

use crate::edit::Form;
use crate::handoff::Handoff;
use crate::import::{Import, Line, Tab};
use crate::sync::{Counts, Ejected, Plan, Sync};
use crate::theme::{BODY, MONO, SANS_SEMIBOLD};

/// The state of the viewer window: what it draws.
enum Viewer {
    /// The library could not be opened. The window shows the error text.
    OpenFailed(String),
    /// The library is open. The window shows the table and, when a book
    /// is selected, the sidebar. The box keeps the enum the size of the
    /// small variant.
    Open(Box<Open>),
}

struct Open {
    /// The open library. It holds the lock for the life of the window,
    /// so a CLI command fails while the window shows the library. It is
    /// None while the import task has it.
    library: Option<Library>,
    /// The library folder. The status bar and the sidebar footer show it.
    folder: PathBuf,
    /// The books in id order. The table sorts a borrowed view.
    books: Vec<Book>,
    /// Reading progress by book id, one row per device.
    progress: BTreeMap<i64, Vec<ProgressRow>>,
    /// Every looked-up word, newest first. The words pane lists them all
    /// and the sidebar lists the selected book's.
    words: Vec<WordRow>,
    /// The open row of the words pane, by word id, and its entries from
    /// Webster's 1913 after `dictionary::plain`. An empty list is a word
    /// the dictionary has no entry for; an error is a dictionary file
    /// that did not unpack. The view reads the entries from here, so it
    /// unpacks nothing while it draws.
    definition: Option<(i64, Result<Vec<String>, String>)>,
    /// The pane in the main area.
    pane: Pane,
    /// The sorted column and the filter field's text, as the core query
    /// the table selects its rows with. The words pane matches the same
    /// text against the word and the book title.
    query: Query,
    /// The scroll offset of the pane in view, in pixels. The pane builds
    /// only the rows in view at that offset.
    scroll: f32,
    /// What the sidebar shows, if it is open.
    sidebar: Option<Sidebar>,
    /// The cover of each book the sidebar has shown, by book id. `Some`
    /// is the thumbnail the library holds; `None` is a book the library
    /// has no cover for, whatever the reason; a missing key is a book
    /// the viewer has not asked about. The map lives for the window
    /// session and holds about 40 KB a book. A removed id never comes
    /// back, so an entry never shows the wrong cover.
    covers: BTreeMap<i64, Option<image::Handle>>,
    /// The full-size cover over the window, while it is open.
    full_cover: Option<image::Handle>,
    /// The book the remove dialog asks about, while the dialog is shown.
    removing: Option<i64>,
    /// The last reload, remove, or open that failed, as one sentence. The
    /// status bar shows it until a reload succeeds.
    error: Option<String>,
    /// The import under way or last done, until the × clears it. While
    /// it is shown, the books pane draws the rows of its tab in view.
    import: Option<Import>,
    /// The query and the scroll offset from before the import, set aside
    /// when a strip takes the pane and put back when the × clears it.
    before: Option<(Query, f32)>,
    /// What the sync pane shows: the plan, where the sync is, and what
    /// the last sync came to. It holds the device while a plan is in
    /// view.
    sync: Sync,
    /// Whether files are held over the window in a drag.
    hovering: bool,
}

/// The pane in the main area: the table of books, the list of words, or
/// the sync plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Books,
    Words,
    Sync,
}

/// What the sidebar shows: a book's details, or the form that edits them.
enum Sidebar {
    Read(Selected),
    Edit(Form),
}

impl Sidebar {
    /// The book in the sidebar, in either view.
    fn id(&self) -> i64 {
        match self {
            Sidebar::Read(selected) => selected.id,
            Sidebar::Edit(form) => form.id,
        }
    }
}

/// The book in the sidebar's read view.
#[derive(Debug, Clone)]
struct Selected {
    id: i64,
    /// The book's description, parsed for the markdown widget.
    description: Vec<markdown::Item>,
}

#[derive(Debug, Clone)]
enum Message {
    /// A click on a row.
    Select(i64),
    /// The sidebar's close button, or the Escape key.
    Close,
    /// A click on a toolbar tab.
    Show(Pane),
    /// A click on a row of the words pane, by word id. It opens the
    /// definition panel under the row, or closes it on the open row.
    Define(i64),
    /// A click on a column header.
    Sort(SortKey),
    /// A change to the filter field.
    Filter(String),
    /// The table body scrolled to this offset in pixels.
    Scrolled(f32),
    /// The Reload button. The viewer reads the library again.
    Reload,
    /// The Import button. The viewer opens the file picker.
    Pick,
    /// The file picker closed with these paths, none on a cancel.
    Picked(Vec<PathBuf>),
    /// Files are held over the window, or the drag left it.
    Hovering(bool),
    /// A file was dropped onto the window.
    Dropped(PathBuf),
    /// The import task finished one file and hands the library back.
    Imported(Handoff<Library>, Line),
    /// The Cancel button on the import strip. The queued files are
    /// dropped; the file in flight finishes.
    CancelImport,
    /// A click on a tab of the import strip.
    ImportTab(Tab),
    /// The × on the import strip. The pane goes back to the query and
    /// the scroll offset from before the import.
    ClearImport,
    /// The Open button in the sidebar. The system reader opens the
    /// selected book's file.
    OpenBook,
    /// A click on a link in the description. The browser opens it.
    OpenLink(String),
    /// The reader or the browser could not be started. The status bar
    /// shows why.
    OpenFailed(String),
    /// A click on the cover in the sidebar.
    ShowCover,
    /// A click on the full-size cover, or Escape.
    HideCover,
    /// The Edit button in the sidebar. The body becomes the form.
    Edit,
    /// A change to one input of the form.
    Form(edit::Change),
    /// The form's Save button, or Enter in one of its single-line
    /// inputs.
    Save,
    /// The form's Cancel button, or Escape. The read view comes back.
    CancelEdit,
    /// The Remove… button in the sidebar. The remove dialog opens on the
    /// selected book.
    AskRemove,
    /// The dialog's Remove button. The book is removed and the library
    /// reads again.
    ConfirmRemove,
    /// The dialog's Cancel button, a click outside the dialog, or Escape.
    CancelRemove,
    /// The Plan button in the sync pane. The viewer looks for the Kobo
    /// and reads the plan. The first visit to the pane does the same.
    Plan,
    /// The planning task hands the library back with the plan, or with
    /// the reason there is none.
    SyncPlanned(Handoff<(Library, Result<Plan, String>)>),
    /// The Sync button in the sync pane. The actions in the plan run.
    RunSync,
    /// The sync task ran one action and hands the library and the device
    /// back.
    Synced(Handoff<(Library, Kobo)>, Result<(), String>),
    /// The sync task wrote the Kobo rows and read progress back.
    SyncRead(Handoff<(Library, Kobo)>, Result<Counts, String>),
    /// The Cancel button in the sync pane. The actions that have not
    /// started are dropped; the one in flight finishes.
    CancelSync,
    /// The Eject button in the sync pane.
    Eject,
    /// The eject task is done.
    Ejected(Ejected),
    /// A frame was drawn while the sync pane's activity bar is in view.
    /// The block on the bar moves.
    Tick(Instant),
}

fn main() -> iced::Result {
    iced::application(boot, update, view)
        .title("EpubSync")
        .window_size((1180.0, 760.0))
        .default_font(theme::SANS)
        .font(include_bytes!("../fonts/instrument-sans/InstrumentSans[wdth,wght].ttf").as_slice())
        .font(include_bytes!("../fonts/newsreader/Newsreader[opsz,wght].ttf").as_slice())
        .font(include_bytes!("../fonts/newsreader/Newsreader-Italic[opsz,wght].ttf").as_slice())
        .font(include_bytes!("../fonts/jetbrains-mono/JetBrainsMono[wght].ttf").as_slice())
        .style(|_viewer, theme| theme::window(theme))
        .subscription(subscription)
        .run()
}

/// Escape closes the sidebar, and a file drag or drop on the window
/// starts an import. The filter field takes Escape first while it has
/// focus, to drop the focus, so only an ignored Escape counts.
///
/// While the sync pane's activity bar is in view, each drawn frame sends
/// a tick, which asks for the next frame. The ticks stop with the task,
/// so an idle window draws nothing.
fn subscription(viewer: &Viewer) -> Subscription<Message> {
    let events = event::listen_with(|event, status, _window| match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key::Named::Escape),
            ..
        }) if status == Status::Ignored => Some(Message::Close),
        Event::Window(window::Event::FileHovered(_)) => Some(Message::Hovering(true)),
        Event::Window(window::Event::FilesHoveredLeft) => Some(Message::Hovering(false)),
        Event::Window(window::Event::FileDropped(path)) => Some(Message::Dropped(path)),
        _ => None,
    });
    let frames = match viewer {
        Viewer::Open(open) if open.sync.animating() => window::frames().map(Message::Tick),
        _ => Subscription::none(),
    };
    Subscription::batch([events, frames])
}

/// The state at start: the open library, or the reason it did not open.
fn boot() -> Viewer {
    match open() {
        Ok(open) => Viewer::Open(Box::new(open)),
        Err(e) => Viewer::OpenFailed(format!("{e:#}")),
    }
}

fn open() -> anyhow::Result<Open> {
    let config = config::load(&config::path()?)?;
    let library = Library::open(&config)?;
    Open::new(library)
}

impl Open {
    /// The state for an open library, with its books, progress, and
    /// words read once.
    fn new(library: Library) -> anyhow::Result<Open> {
        let mut open = Open {
            folder: library.folder.clone(),
            library: Some(library),
            books: Vec::new(),
            progress: BTreeMap::new(),
            words: Vec::new(),
            definition: None,
            pane: Pane::Books,
            query: Query::default(),
            scroll: 0.0,
            sidebar: None,
            covers: BTreeMap::new(),
            full_cover: None,
            removing: None,
            error: None,
            import: None,
            before: None,
            sync: Sync::new(),
            hovering: false,
        };
        open.read()?;
        Ok(open)
    }

    /// Reads the books, the progress, and the words from the library.
    /// The state changes only when all three reads succeed.
    fn read(&mut self) -> anyhow::Result<()> {
        let Some(library) = &self.library else {
            anyhow::bail!("the import task holds the library");
        };
        let books = library.list()?;
        let progress = library.progress()?;
        let words = library.words(None, None)?;
        self.books = books;
        self.progress = progress;
        self.words = words;
        Ok(())
    }

    /// Reads the library again. The sort, the filter, and the scroll
    /// offset stay. The sidebar stays on its book, and closes when the
    /// book is gone. A read that fails keeps the rows from the last
    /// read and puts the error in the status bar.
    ///
    /// The read view gets its description parsed again. The form keeps
    /// the text the reader typed, so a reload during an edit throws
    /// none of it away.
    ///
    /// The viewer holds the library lock, so no other process writes
    /// while the window is open. A write from the viewer ends with a
    /// reload.
    fn reload(&mut self) {
        self.error = self.read().err().map(|e| format!("Reload failed: {e:#}"));
        let Some(id) = self.sidebar.as_ref().map(Sidebar::id) else {
            return;
        };
        match self.sidebar {
            Some(Sidebar::Read(_)) => self.select(id),
            Some(Sidebar::Edit(_)) if !self.books.iter().any(|b| b.id == id) => {
                self.sidebar = None;
            }
            _ => {}
        }
    }

    /// Puts the form in the sidebar on the book it shows. A sidebar
    /// that already shows the form, or no sidebar, changes nothing.
    fn edit(&mut self) {
        let Some(Sidebar::Read(selected)) = &self.sidebar else {
            return;
        };
        let id = selected.id;
        if let Some(book) = self.books.iter().find(|b| b.id == id) {
            self.sidebar = Some(Sidebar::Edit(Form::new(book)));
        }
    }

    /// Writes the form's record to the library and puts the read view
    /// back. A record the inputs do not make, and a write that fails,
    /// keep the form and put one sentence in its error line.
    ///
    /// The library reads again either way, because `Library::edit`
    /// writes the row before the file, and a file write that fails
    /// still leaves a changed row.
    fn save(&mut self) {
        // The Save button is off while the import task holds the
        // library, so this guards a stale message only.
        if self.library.is_none() {
            return;
        }
        let Some(Sidebar::Edit(form)) = &mut self.sidebar else {
            return;
        };
        let id = form.id;
        let record = match form.record() {
            Ok(record) => record,
            Err(why) => {
                form.error = Some(why);
                return;
            }
        };
        let result = self
            .library
            .as_mut()
            .expect("the library is here")
            .edit(id, &record);
        self.reload();
        match result {
            Ok(()) => self.select(id),
            Err(e) => {
                if let Some(Sidebar::Edit(form)) = &mut self.sidebar {
                    form.error = Some(format!("Save failed: {e:#}"));
                }
            }
        }
    }

    /// Drops the form and puts the read view back on its book.
    fn cancel_edit(&mut self) {
        let Some(Sidebar::Edit(form)) = &self.sidebar else {
            return;
        };
        let id = form.id;
        self.select(id);
    }

    /// Removes a book and reads the library again, which closes the
    /// sidebar when it showed that book. A remove that fails puts its
    /// error in the status bar after the reload, so the error stays. The
    /// dialog's Remove button is off while the import task holds the
    /// library, so a missing library is not an error here.
    fn remove(&mut self, id: i64) {
        let Some(library) = &mut self.library else {
            return;
        };
        let result = library.remove(id);
        self.reload();
        if let Err(e) = result {
            self.error = Some(format!("Remove failed: {e:#}"));
        }
    }

    /// Opens the selected book's file in the system reader, or does
    /// nothing while no book is selected.
    fn open_book(&self) -> Task<Message> {
        let Some(id) = self.sidebar.as_ref().map(Sidebar::id) else {
            return Task::none();
        };
        let path = self.folder.join(book_file_name(id));
        launch("open the book", move || opener::open(path))
    }

    /// Puts a book's read view in the sidebar, in place of the form
    /// when the form is shown. Its description is parsed here and its
    /// cover is read here, so a book that is never shown costs neither.
    /// An id no book has closes the sidebar.
    fn select(&mut self, id: i64) {
        self.sidebar = self.books.iter().find(|b| b.id == id).map(|b| {
            let html = b.metadata.description.as_deref().unwrap_or("");
            Sidebar::Read(Selected {
                id,
                description: description::parse(html),
            })
        });
        if self.sidebar.is_some() {
            self.read_cover(id);
        }
    }

    /// Reads a book's cover into the map, once per book. It reads one
    /// row and opens no file.
    ///
    /// A book the library has not read yet gets no entry, and the next
    /// click asks again. The read is skipped while the import task
    /// holds the library; the reload after each imported file calls
    /// `select` again, and that call fills the entry.
    fn read_cover(&mut self, id: i64) {
        if self.covers.contains_key(&id) {
            return;
        }
        let Some(library) = &self.library else {
            return;
        };
        match library.cover(id) {
            Ok(Cover::Image(bytes)) => {
                self.covers
                    .insert(id, Some(image::Handle::from_bytes(bytes)));
            }
            // The reader can do nothing about any of the three, so the
            // sidebar draws them the same. The state and its text stay
            // in the database for `check`.
            Ok(Cover::None | Cover::Unreadable(_) | Cover::Undecodable(_)) => {
                self.covers.insert(id, None);
            }
            Ok(Cover::Unknown) => {}
            Err(e) => self.error = Some(format!("Cover failed: {e:#}")),
        }
    }

    /// Opens the definition panel under the word row `id`, or closes it
    /// when `id` is the open row. Only one row is open at a time. A word
    /// from a dictionary other than English gets no lookup and shows as
    /// a word with no entry. The first lookup parses the dictionary
    /// index.
    fn define(&mut self, id: i64) {
        if self
            .definition
            .as_ref()
            .is_some_and(|(open, _)| *open == id)
        {
            self.definition = None;
            return;
        }
        let Some(row) = self.words.iter().find(|w| w.id == id) else {
            return;
        };
        let entries = if words::english(row) {
            dictionary::define(&row.word)
                .map(|entries| entries.iter().map(|e| dictionary::plain(e)).collect())
                .map_err(|e| format!("{e:#}"))
        } else {
            Ok(Vec::new())
        };
        self.definition = Some((id, entries));
    }

    /// Reads the selected book's file and puts its cover over the
    /// window at the size the publisher stored.
    fn show_cover(&mut self) {
        let Some(id) = self.sidebar.as_ref().map(Sidebar::id) else {
            return;
        };
        let Some(library) = &self.library else {
            return;
        };
        let read = library.full_cover(id);
        match read {
            Ok(FileCover::Image(bytes)) => {
                self.full_cover = Some(image::Handle::from_bytes(bytes));
            }
            Ok(FileCover::None) => self.error = Some("The book file gives no cover.".to_string()),
            Ok(FileCover::Unreadable(why)) => self.error = Some(format!("Cover failed: {why}")),
            Err(e) => self.error = Some(format!("Cover failed: {e:#}")),
        }
    }

    /// Queues paths for import and starts the task if it is idle. Paths
    /// that arrive while an import runs join its queue. Paths that
    /// arrive after one ended start a new strip in place of the old.
    ///
    /// The sync task holds the library while it runs, so paths that
    /// arrive then are refused with a sentence in the status bar rather
    /// than queued into a strip that cannot start.
    fn import(&mut self, paths: &[PathBuf]) -> Task<Message> {
        if self.sync.running() {
            self.error = Some("The sync is running. Import when it ends.".to_string());
            return Task::none();
        }
        let mut task = Task::none();
        if !self.import.as_ref().is_some_and(Import::running) {
            task = self.begin();
        }
        let import = self.import.as_mut().expect("the strip is in place");
        for path in paths {
            import.add(path);
        }
        Task::batch([task, import.start(&mut self.library)])
    }

    /// Puts a new strip in place of the old and gives it the pane. The
    /// query and the scroll offset go aside for the × to put back, unless
    /// an earlier strip put them aside already. The pane starts at the
    /// top, sorted by id, which is import order.
    fn begin(&mut self) -> Task<Message> {
        self.import = Some(Import::new());
        if self.before.is_none() {
            self.before = Some((std::mem::take(&mut self.query), self.scroll));
        }
        self.query = Query {
            sort: Sort::by(SortKey::Id),
            ..Query::default()
        };
        self.scroll_home()
    }

    /// Puts a pane in the main area, at the top. The panes share one
    /// scrollable, so a new pane starts at the top rather than at the old
    /// pane's offset.
    fn show(&mut self, pane: Pane) -> Task<Message> {
        self.pane = pane;
        self.scroll_home()
    }

    /// Scrolls the pane in view to the top.
    fn scroll_home(&mut self) -> Task<Message> {
        self.scroll = 0.0;
        table::scroll_to_top()
    }
}

fn update(viewer: &mut Viewer, message: Message) -> Task<Message> {
    let Viewer::Open(open) = viewer else {
        return Task::none();
    };
    match message {
        Message::Select(id) => open.select(id),
        Message::Define(id) => open.define(id),
        // Escape reaches Close while the dialog is shown, and cancels it.
        Message::Close if open.removing.is_some() => open.removing = None,
        Message::Close if open.full_cover.is_some() => open.full_cover = None,
        // A focused text input takes the first Escape to drop its
        // focus, so the second one reaches here and cancels the edit.
        // The × in the header sends Close too, so a first click on it
        // cancels the edit and a second closes the sidebar.
        Message::Close if matches!(open.sidebar, Some(Sidebar::Edit(_))) => open.cancel_edit(),
        Message::Close => open.sidebar = None,
        Message::Show(pane) => {
            // A click on the Sync tab with no plan in view looks for the
            // Kobo, so the plan is there without a second click.
            let plan = if pane == Pane::Sync && open.sync.idle() {
                open.sync.make_plan(&mut open.library)
            } else {
                Task::none()
            };
            if open.pane != pane {
                return Task::batch([open.show(pane), plan]);
            }
            return plan;
        }
        Message::Sort(key) => {
            let sort = &mut open.query.sort;
            if sort.keys == [key] {
                sort.descending = !sort.descending;
            } else {
                *sort = Sort::by(key);
            }
        }
        Message::Filter(text) => {
            // A new filter shows its matches from the top.
            open.query.filter.text = text;
            return open.scroll_home();
        }
        Message::Scrolled(offset) => open.scroll = offset,
        Message::Reload => open.reload(),
        Message::Pick => return import::pick(),
        // A cancelled picker gives no paths and shows nothing.
        Message::Picked(paths) if !paths.is_empty() => return open.import(&paths),
        Message::Picked(_) => {}
        Message::Hovering(hovering) => open.hovering = hovering,
        Message::Dropped(path) => {
            open.hovering = false;
            return open.import(&[path]);
        }
        Message::Imported(handoff, line) => {
            open.library = handoff.take();
            if let Some(import) = &mut open.import {
                import.finish(line);
            }
            open.reload();
            if let Some(import) = &mut open.import {
                return import.start(&mut open.library);
            }
        }
        Message::CancelImport => {
            if let Some(import) = &mut open.import {
                import.cancel();
            }
        }
        Message::ImportTab(tab) => {
            // Each tab shows its rows from the top, in the books pane.
            if let Some(import) = &mut open.import {
                import.show(tab);
                return open.show(Pane::Books);
            }
        }
        Message::ClearImport => {
            open.import = None;
            if let Some((query, scroll)) = open.before.take() {
                open.query = query;
                open.scroll = scroll;
                return table::scroll_to(scroll);
            }
        }
        Message::OpenBook => return open.open_book(),
        Message::OpenLink(uri) => {
            return launch("open the link", move || opener::open_browser(uri));
        }
        Message::OpenFailed(error) => open.error = Some(error),
        Message::ShowCover => open.show_cover(),
        Message::HideCover => open.full_cover = None,
        Message::Edit => open.edit(),
        Message::Form(change) => {
            if let Some(Sidebar::Edit(form)) = &mut open.sidebar {
                form.apply(change);
            }
        }
        Message::Save => open.save(),
        Message::CancelEdit => open.cancel_edit(),
        Message::AskRemove => open.removing = open.sidebar.as_ref().map(Sidebar::id),
        Message::ConfirmRemove => {
            if let Some(id) = open.removing.take() {
                open.remove(id);
            }
        }
        Message::CancelRemove => open.removing = None,
        Message::Plan => return open.sync.make_plan(&mut open.library),
        Message::SyncPlanned(handoff) => open.library = open.sync.planned(handoff.take()),
        Message::RunSync => return open.sync.run(&mut open.library),
        Message::Synced(handoff, result) => {
            open.library = open.sync.synced(handoff.take(), result);
            return open.sync.step(&mut open.library);
        }
        Message::SyncRead(handoff, counts) => {
            open.library = open.sync.read(handoff.take(), counts);
            // The read back writes progress and words, so the panes read
            // the library again.
            open.reload();
        }
        Message::CancelSync => open.sync.cancel(),
        Message::Eject => return open.sync.eject(),
        Message::Ejected(ejected) => open.sync.ejected(ejected),
        Message::Tick(at) => open.sync.tick(at),
    }
    Task::none()
}

/// Runs one of the `opener` calls on a background task, because on macOS
/// they wait for the `open` command to exit. A failure comes back as
/// `Message::OpenFailed`, worded "Could not open the book: …"; a success
/// sends nothing.
fn launch(
    what: &'static str,
    call: impl FnOnce() -> Result<(), opener::OpenError> + Send + 'static,
) -> Task<Message> {
    Task::future(async move { call() }).then(move |result| match result {
        Ok(()) => Task::none(),
        Err(e) => Task::done(Message::OpenFailed(format!("Could not {what}: {e}"))),
    })
}

fn view(viewer: &Viewer) -> Element<'_, Message> {
    match viewer {
        Viewer::OpenFailed(error) => container(text(error)).padding(16).into(),
        Viewer::Open(open) => {
            let (pane, shown_count): (Element<'_, Message>, usize) = match open.pane {
                // The import strip, while shown, gives the books pane the
                // rows of its tab in view.
                Pane::Books => match &open.import {
                    Some(import) => import::pane(open, import),
                    None => {
                        let rows = open.query.select(&open.books, &open.progress);
                        let n = rows.len();
                        (table::view(open, rows), n)
                    }
                },
                Pane::Words => {
                    let rows = words::select(&open.words, &open.query.filter.text);
                    let n = rows.len();
                    (words::view(open, rows), n)
                }
                Pane::Sync => (sync::view(open, &open.sync), open.sync.len()),
            };
            let shown = open.sidebar.as_ref().and_then(|s| {
                let book = open.books.iter().find(|b| b.id == s.id())?;
                Some((book, s))
            });
            let mut main = row![pane].height(Fill);
            if let Some((book, sidebar)) = shown {
                main = main.push(detail::view(open, book, sidebar));
            }
            // The import strip sits under the toolbar. A drag over the
            // window shows the drop hint there while no import is shown.
            let strip = match &open.import {
                Some(import) => Some(import::view(import)),
                None if open.hovering => Some(import::drop_hint()),
                None => None,
            };
            let window: Element<'_, Message> = column![toolbar(open, shown_count)]
                .extend(strip)
                .push(main)
                .push(status_bar(open))
                .into();
            let window = match &open.full_cover {
                Some(handle) => full_cover::over(window, handle),
                None => window,
            };
            let removing = open
                .removing
                .and_then(|id| open.books.iter().find(|b| b.id == id));
            match removing {
                Some(book) => remove::over(window, book, open.library.is_some()),
                None => window,
            }
        }
    }
}

/// "1 book", "23 books", "1 word", "12 words".
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// The toolbar: the Library, Words, and Sync tabs, the count for the pane
/// in view, the filter field, and the Import… and Reload buttons. The
/// count reads "4 of 23 books" while the filter is set, and is empty on
/// the sync pane while it holds no plan. The filter field is out of the
/// toolbar on the sync pane, which it does not narrow. Reload is off while
/// the import or a sync task holds the library, and Import… is off while
/// a sync task holds it.
fn toolbar<'a>(open: &'a Open, shown: usize) -> Element<'a, Message> {
    let filtered = |total: usize, noun: &str| {
        if open.query.filter.is_empty() {
            count(total, noun)
        } else {
            format!("{shown} of {}", count(total, noun))
        }
    };
    let (count, placeholder) = match open.pane {
        Pane::Books => (
            filtered(open.books.len(), "book"),
            Some("Filter by title, author, or series"),
        ),
        Pane::Words => (
            filtered(open.words.len(), "word"),
            Some("Filter by word or book"),
        ),
        Pane::Sync if shown == 0 => (String::new(), None),
        Pane::Sync => (count(shown, "action"), None),
    };
    let tab = |label: &'static str, pane: Pane| {
        button(text(label).size(14).font(SANS_SEMIBOLD))
            .on_press(Message::Show(pane))
            .padding(0)
            .style(theme::tab(open.pane == pane))
    };
    let filter = placeholder.map(|placeholder| {
        Element::from(
            text_input(placeholder, &open.query.filter.text)
                .on_input(Message::Filter)
                .width(300)
                .size(13)
                .padding([5, 10])
                .style(theme::filter),
        )
    });
    let action = |label: &'static str, message: Option<Message>| {
        button(text(label).size(13))
            .on_press_maybe(message)
            .padding([5, 10])
            .style(theme::action)
    };
    let bar = row![
        tab("Library", Pane::Books),
        tab("Words", Pane::Words),
        tab("Sync", Pane::Sync),
        text(count).size(BODY).style(theme::text_color(|c| c.muted)),
        space().width(Fill),
    ]
    .extend(filter)
    .push(action(
        "Import…",
        (!open.sync.running()).then_some(Message::Pick),
    ))
    .push(action(
        "Reload",
        open.library.is_some().then_some(Message::Reload),
    ))
    .spacing(14)
    .align_y(Center)
    .height(46)
    .padding(padding::horizontal(14));
    column![bar, theme::hline()].into()
}

/// The status bar: the count, how many books are reading and finished by
/// the progress row read last, how many were finished this year by
/// their finished date, and the library folder. The error of a failed
/// reload, remove, or open takes the place of the counts. A click on the
/// folder opens it in the system file manager.
fn status_bar(open: &Open) -> Element<'_, Message> {
    let has_status = |status: ReadStatus| {
        open.books
            .iter()
            .filter(|b| query::status(&open.progress, b.id) == status)
            .count()
    };
    let year = format::this_year();
    let this_year = open
        .books
        .iter()
        .filter(|b| query::finished(&open.progress, b.id).is_some_and(|d| d.starts_with(&year)))
        .count();
    let counts = match &open.error {
        Some(error) => text(error).size(11.5).style(theme::text_color(|c| c.ink)),
        None => text(format!(
            "{} reading · {} finished · {this_year} this year",
            has_status(ReadStatus::Reading),
            has_status(ReadStatus::Finished)
        ))
        .size(11.5)
        .style(theme::text_color(|c| c.muted)),
    };
    let bar = row![
        text(count(open.books.len(), "book"))
            .size(11.5)
            .style(theme::text_color(|c| c.muted)),
        counts,
        space().width(Fill),
        text(open.folder.display().to_string())
            .font(MONO)
            .size(11)
            .wrapping(text::Wrapping::None)
            .style(theme::text_color(|c| c.muted)),
    ]
    .spacing(18)
    .align_y(Center)
    .height(28)
    .padding(padding::horizontal(14));
    column![theme::hline(), bar].into()
}

#[cfg(test)]
mod tests {
    use epubsync_core::library::ImportOutcome;
    use epubsync_epub::fixtures;

    use super::*;

    /// A viewer on a library with one imported book, shown in the
    /// sidebar, and that book's id.
    fn with_one_book() -> (tempfile::TempDir, Viewer, i64) {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::init(&dir.path().join("library")).unwrap();
        let epub = fixtures::write_epub(dir.path(), "lhod.epub", fixtures::EPUB2_OPF);
        let ImportOutcome::Imported { id, .. } = lib.import(&epub, false).unwrap() else {
            panic!("not imported");
        };
        let mut open = Open::new(lib).unwrap();
        open.select(id);
        (dir, Viewer::Open(Box::new(open)), id)
    }

    fn state(viewer: &Viewer) -> &Open {
        let Viewer::Open(open) = viewer else {
            panic!("the library did not open");
        };
        open
    }

    /// The form the sidebar shows, or a panic when it shows anything
    /// else.
    fn form(viewer: &Viewer) -> &Form {
        let Some(Sidebar::Edit(form)) = &state(viewer).sidebar else {
            panic!("the sidebar does not show the form");
        };
        form
    }

    fn title(viewer: &Viewer) -> &str {
        &state(viewer).books[0].metadata.title
    }

    fn revision(viewer: &Viewer) -> i64 {
        state(viewer).books[0].revision
    }

    fn typed(title: &str) -> Message {
        Message::Form(edit::Change::Title(title.to_string()))
    }

    #[test]
    fn selecting_a_book_puts_its_cover_in_the_map() {
        let (_dir, viewer, id) = with_one_book();
        let open = state(&viewer);
        assert!(open.covers[&id].is_some());
        assert_eq!(open.error, None);
    }

    #[test]
    fn a_click_on_the_cover_shows_the_full_size_one_and_escape_closes_it() {
        let (_dir, mut viewer, id) = with_one_book();
        let _ = update(&mut viewer, Message::ShowCover);
        let open = state(&viewer);
        assert!(open.full_cover.is_some());
        assert_eq!(open.error, None);

        let _ = update(&mut viewer, Message::Close);
        let open = state(&viewer);
        assert!(open.full_cover.is_none());
        assert_eq!(open.sidebar.as_ref().map(Sidebar::id), Some(id));
    }

    #[test]
    fn confirm_remove_deletes_the_book_and_closes_the_sidebar() {
        let (_dir, mut viewer, id) = with_one_book();
        let path = state(&viewer).folder.join(book_file_name(id));
        assert!(path.exists());

        let _ = update(&mut viewer, Message::AskRemove);
        assert_eq!(state(&viewer).removing, Some(id));

        let _ = update(&mut viewer, Message::ConfirmRemove);
        let open = state(&viewer);
        assert_eq!(open.removing, None);
        assert!(open.books.is_empty());
        assert!(open.sidebar.is_none());
        assert_eq!(open.error, None);
        assert!(!path.exists());
    }

    #[test]
    fn escape_cancels_the_dialog_and_keeps_the_book() {
        let (_dir, mut viewer, id) = with_one_book();
        let _ = update(&mut viewer, Message::AskRemove);
        let _ = update(&mut viewer, Message::Close);
        let open = state(&viewer);
        assert_eq!(open.removing, None);
        assert_eq!(open.sidebar.as_ref().map(Sidebar::id), Some(id));
        assert_eq!(open.books.len(), 1);
    }

    #[test]
    fn remove_is_skipped_while_the_import_task_holds_the_library() {
        let (_dir, mut viewer, _id) = with_one_book();
        let Viewer::Open(open) = &mut viewer else {
            panic!("the library did not open");
        };
        let library = open.library.take();
        let _ = update(&mut viewer, Message::AskRemove);
        let _ = update(&mut viewer, Message::ConfirmRemove);
        let open = state(&viewer);
        assert_eq!(open.removing, None);
        assert_eq!(open.books.len(), 1);
        drop(library);
    }

    #[test]
    fn edit_then_save_writes_the_record_and_shows_the_read_view() {
        let (_dir, mut viewer, id) = with_one_book();
        let before = revision(&viewer);

        let _ = update(&mut viewer, Message::Edit);
        let _ = update(&mut viewer, typed("Renamed"));
        let _ = update(&mut viewer, Message::Save);

        let open = state(&viewer);
        assert_eq!(open.books[0].metadata.title, "Renamed");
        assert_eq!(open.books[0].revision, before + 1);
        assert!(matches!(open.sidebar, Some(Sidebar::Read(_))));
        assert_eq!(open.sidebar.as_ref().map(Sidebar::id), Some(id));
        assert_eq!(open.error, None);
    }

    #[test]
    fn save_with_an_empty_title_keeps_the_form() {
        let (_dir, mut viewer, _id) = with_one_book();
        let before = revision(&viewer);

        let _ = update(&mut viewer, Message::Edit);
        let _ = update(&mut viewer, typed("   "));
        let _ = update(&mut viewer, Message::Save);

        assert_eq!(form(&viewer).error.as_deref(), Some("The title is empty."));
        assert_eq!(revision(&viewer), before);
    }

    #[test]
    fn escape_cancels_the_edit_and_keeps_the_book() {
        let (_dir, mut viewer, id) = with_one_book();
        let before = title(&viewer).to_string();

        let _ = update(&mut viewer, Message::Edit);
        let _ = update(&mut viewer, typed("Renamed"));
        let _ = update(&mut viewer, Message::Close);

        let open = state(&viewer);
        assert!(matches!(open.sidebar, Some(Sidebar::Read(_))));
        assert_eq!(open.sidebar.as_ref().map(Sidebar::id), Some(id));
        assert_eq!(open.books[0].metadata.title, before);
    }

    #[test]
    fn reload_keeps_the_form() {
        let (_dir, mut viewer, _id) = with_one_book();
        let _ = update(&mut viewer, Message::Edit);
        let _ = update(&mut viewer, typed("Typed"));
        let _ = update(&mut viewer, Message::Reload);
        assert_eq!(form(&viewer).title, "Typed");
    }

    /// The first click on the Sync tab starts the planning task, which
    /// takes the library. A second click while it is out starts nothing.
    #[test]
    fn the_first_visit_to_the_sync_tab_starts_the_plan() {
        let (_dir, mut viewer, _id) = with_one_book();
        let _ = update(&mut viewer, Message::Show(Pane::Sync));
        let open = state(&viewer);
        assert_eq!(open.pane, Pane::Sync);
        assert!(open.sync.running());
        assert!(open.sync.animating());
        assert!(open.library.is_none());

        let _ = update(&mut viewer, Message::Show(Pane::Books));
        let _ = update(&mut viewer, Message::Show(Pane::Sync));
        assert!(state(&viewer).sync.running());
    }

    /// The plan the task hands back fills the pane and gives the library
    /// back, and a later visit to the tab keeps the plan. The plan is
    /// built here, since the task scans the mount folders of the machine
    /// the test runs on.
    #[test]
    fn a_plan_fills_the_sync_pane_and_stays_on_a_later_visit() {
        let (dir, mut viewer, _id) = with_one_book();
        let root = sync::library_tests::fake_kobo(dir.path());
        let Viewer::Open(open) = &mut viewer else {
            panic!("the library did not open");
        };
        let library = open.library.take().expect("the library is here");
        let plan = sync::plan(&library, Kobo::at(&root).unwrap()).unwrap();

        let handoff = Handoff::new((library, Ok(plan)));
        let _ = update(&mut viewer, Message::SyncPlanned(handoff));
        let open = state(&viewer);
        assert!(open.library.is_some());
        assert_eq!(open.sync.len(), 1);
        assert!(!open.sync.running());

        let _ = update(&mut viewer, Message::Show(Pane::Sync));
        let open = state(&viewer);
        assert!(open.library.is_some());
        assert_eq!(open.sync.len(), 1);
    }

    #[test]
    fn save_is_skipped_while_the_import_task_holds_the_library() {
        let (_dir, mut viewer, _id) = with_one_book();
        let before = title(&viewer).to_string();
        let Viewer::Open(open) = &mut viewer else {
            panic!("the library did not open");
        };
        let library = open.library.take();

        let _ = update(&mut viewer, Message::Edit);
        let _ = update(&mut viewer, typed("Renamed"));
        let _ = update(&mut viewer, Message::Save);

        assert!(matches!(state(&viewer).sidebar, Some(Sidebar::Edit(_))));
        assert_eq!(title(&viewer), before);
        let Viewer::Open(open) = &mut viewer else {
            panic!("the library did not open");
        };
        open.library = library;
    }
}
