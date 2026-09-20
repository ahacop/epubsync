//! The library viewer: a window with a table of books and, when a book is
//! selected, its cover and its details in a sidebar on the right. A Words
//! tab swaps the table for the list of words looked up on a device. A
//! Reload button reads the library again, and an Import button or a drop
//! of files onto the window adds books, with a strip under the toolbar
//! that shows the progress and gives the table the added books. A Remove…
//! button in the sidebar removes the selected book after a dialog, and an
//! Open button there opens the book in the system reader. A click on the
//! cover there opens it at full size over the window. A link in a
//! description opens in the browser. The CLI edits and syncs.

mod description;
mod detail;
mod format;
mod full_cover;
mod import;
mod remove;
mod table;
mod theme;
mod words;

use std::collections::BTreeMap;
use std::path::PathBuf;

use epubsync_core::config;
use epubsync_core::cover::Cover;
use epubsync_core::device::ReadStatus;
use epubsync_core::library::{Book, FileCover, Library, ProgressRow, WordRow, book_file_name};
use epubsync_core::query::{self, Query, Sort, SortKey};
use iced::event::{self, Event, Status};
use iced::keyboard::{self, key};
use iced::widget::{button, column, container, image, markdown, row, space, text, text_input};
use iced::{Center, Element, Fill, Subscription, Task, padding, window};

use crate::import::{Handoff, Import, Line, Tab};
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
    /// The pane in the main area.
    pane: Pane,
    /// The sorted column and the filter field's text, as the core query
    /// the table selects its rows with. The words pane matches the same
    /// text against the word and the book title.
    query: Query,
    /// The scroll offset of the pane in view, in pixels. The pane builds
    /// only the rows in view at that offset.
    scroll: f32,
    /// The book in the sidebar, if any.
    selected: Option<Selected>,
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
    /// Whether files are held over the window in a drag.
    hovering: bool,
}

/// The pane in the main area: the table of books, or the list of words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Books,
    Words,
}

/// The book in the sidebar.
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
    Imported(Handoff, Line),
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
    /// The Remove… button in the sidebar. The remove dialog opens on the
    /// selected book.
    AskRemove,
    /// The dialog's Remove button. The book is removed and the library
    /// reads again.
    ConfirmRemove,
    /// The dialog's Cancel button, a click outside the dialog, or Escape.
    CancelRemove,
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
fn subscription(_viewer: &Viewer) -> Subscription<Message> {
    event::listen_with(|event, status, _window| match event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: keyboard::Key::Named(key::Named::Escape),
            ..
        }) if status == Status::Ignored => Some(Message::Close),
        Event::Window(window::Event::FileHovered(_)) => Some(Message::Hovering(true)),
        Event::Window(window::Event::FilesHoveredLeft) => Some(Message::Hovering(false)),
        Event::Window(window::Event::FileDropped(path)) => Some(Message::Dropped(path)),
        _ => None,
    })
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
            pane: Pane::Books,
            query: Query::default(),
            scroll: 0.0,
            selected: None,
            covers: BTreeMap::new(),
            full_cover: None,
            removing: None,
            error: None,
            import: None,
            before: None,
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
    /// offset stay. The sidebar stays on its book with the description
    /// parsed again, and closes when the book is gone. A read that
    /// fails keeps the rows from the last read and puts the error in
    /// the status bar.
    ///
    /// The viewer holds the library lock, so no other process writes
    /// while the window is open. A write from the viewer ends with a
    /// reload.
    fn reload(&mut self) {
        self.error = self.read().err().map(|e| format!("Reload failed: {e:#}"));
        if let Some(id) = self.selected.as_ref().map(|s| s.id) {
            self.select(id);
        }
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
        let Some(selected) = &self.selected else {
            return Task::none();
        };
        let path = self.folder.join(book_file_name(selected.id));
        launch("open the book", move || opener::open(path))
    }

    /// Puts the book in the sidebar. Its description is parsed here and
    /// its cover is read here, so a book that is never shown costs
    /// neither. An id no book has closes the sidebar.
    fn select(&mut self, id: i64) {
        self.selected = self.books.iter().find(|b| b.id == id).map(|b| {
            let html = b.metadata.description.as_deref().unwrap_or("");
            Selected {
                id,
                description: description::parse(html),
            }
        });
        if self.selected.is_some() {
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

    /// Reads the selected book's file and puts its cover over the
    /// window at the size the publisher stored.
    fn show_cover(&mut self) {
        let Some(id) = self.selected.as_ref().map(|s| s.id) else {
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
    fn import(&mut self, paths: &[PathBuf]) -> Task<Message> {
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
        // Escape reaches Close while the dialog is shown, and cancels it.
        Message::Close if open.removing.is_some() => open.removing = None,
        Message::Close if open.full_cover.is_some() => open.full_cover = None,
        Message::Close => open.selected = None,
        Message::Show(pane) => {
            // The two panes share one scrollable id, so the new pane
            // starts at the top rather than at the old pane's offset.
            if open.pane != pane {
                open.pane = pane;
                open.scroll = 0.0;
                return table::scroll_to_top();
            }
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
            open.scroll = 0.0;
            return table::scroll_to_top();
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
                open.pane = Pane::Books;
                open.scroll = 0.0;
                return table::scroll_to_top();
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
        Message::AskRemove => open.removing = open.selected.as_ref().map(|s| s.id),
        Message::ConfirmRemove => {
            if let Some(id) = open.removing.take() {
                open.remove(id);
            }
        }
        Message::CancelRemove => open.removing = None,
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
            };
            let shown = open.selected.as_ref().and_then(|s| {
                let book = open.books.iter().find(|b| b.id == s.id)?;
                Some((book, s))
            });
            let mut main = row![pane].height(Fill);
            if let Some((book, selected)) = shown {
                main = main.push(detail::view(open, book, selected));
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

/// The toolbar: the Library and Words tabs, the count for the pane in
/// view, the filter field, and the Import and Reload buttons. The count
/// reads "4 of 23 books" while the filter is set. Reload is off while
/// the import task holds the library.
fn toolbar<'a>(open: &'a Open, shown: usize) -> Element<'a, Message> {
    let (total, noun, placeholder) = match open.pane {
        Pane::Books => (
            open.books.len(),
            "book",
            "Filter by title, author, or series",
        ),
        Pane::Words => (open.words.len(), "word", "Filter by word or book"),
    };
    let count = if open.query.filter.is_empty() {
        count(total, noun)
    } else {
        format!("{shown} of {}", count(total, noun))
    };
    let tab = |label: &'static str, pane: Pane| {
        button(text(label).size(14).font(SANS_SEMIBOLD))
            .on_press(Message::Show(pane))
            .padding(0)
            .style(theme::tab(open.pane == pane))
    };
    let filter = text_input(placeholder, &open.query.filter.text)
        .on_input(Message::Filter)
        .width(300)
        .size(13)
        .padding([5, 10])
        .style(theme::filter);
    let action = |label: &'static str, message: Option<Message>| {
        button(text(label).size(13))
            .on_press_maybe(message)
            .padding([5, 10])
            .style(theme::action)
    };
    let bar = row![
        tab("Library", Pane::Books),
        tab("Words", Pane::Words),
        text(count).size(BODY).style(theme::text_color(|c| c.muted)),
        space().width(Fill),
        filter,
        action("Import…", Some(Message::Pick)),
        action("Reload", open.library.is_some().then_some(Message::Reload)),
    ]
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
        assert_eq!(open.selected.as_ref().map(|s| s.id), Some(id));
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
        assert!(open.selected.is_none());
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
        assert_eq!(open.selected.as_ref().map(|s| s.id), Some(id));
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
}
