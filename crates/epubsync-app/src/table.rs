//! The table of books: the columns and the table view. Each column is one
//! sort key of the core query, and a click on its header sorts by it. The
//! import's failed table, a file and an error per row, is here too, since
//! it shares the row height and the frame.

use std::collections::BTreeMap;
use std::path::Path;

use epubsync_core::device::ReadStatus;
use epubsync_core::library::{Book, ProgressRow};
use epubsync_core::query::{self, Sort, SortKey};
use iced::widget::{
    self, Text, button, column, container, progress_bar, responsive, row, scrollable, space, text,
};
use iced::{Center, Element, Fill, Length, Right, Size, Task, padding};

use crate::theme::{self, BODY, MONO, SANS_MEDIUM};
use crate::{Message, Open, Sidebar, format};

/// The columns from left to right.
const COLUMNS: [SortKey; 9] = [
    SortKey::Id,
    SortKey::Title,
    SortKey::Author,
    SortKey::Series,
    SortKey::Words,
    SortKey::Ease,
    SortKey::Progress,
    SortKey::LastRead,
    SortKey::Finished,
];

/// The header text of a column.
fn name(column: SortKey) -> &'static str {
    match column {
        SortKey::Id => "ID",
        SortKey::Title => "Title",
        SortKey::Author => "Author",
        SortKey::Series => "Series",
        SortKey::Words => "Words",
        SortKey::Ease => "Ease",
        SortKey::Progress => "Progress",
        SortKey::LastRead => "Last read",
        SortKey::Finished => "Finished",
    }
}

/// The fixed columns take pixels; the text columns share the rest.
fn width(column: SortKey) -> Length {
    match column {
        SortKey::Id => Length::Fixed(56.0),
        SortKey::Title => Length::FillPortion(32),
        SortKey::Author => Length::FillPortion(20),
        SortKey::Series => Length::FillPortion(18),
        SortKey::Words => Length::Fixed(88.0),
        SortKey::Ease => Length::Fixed(64.0),
        SortKey::Progress => Length::Fixed(200.0),
        SortKey::LastRead | SortKey::Finished => Length::Fixed(108.0),
    }
}

/// The width of the selected row's mark on the left edge. Every row and
/// the header row leave this space, so the cells line up.
pub const MARK: f32 = 3.0;

/// The height of one row, and the pitch from one row to the next: the
/// row and the 1 px line under it. The table builds only the rows in
/// view, and the pitch tells it which rows those are. The words pane and
/// the failed table share both.
pub const ROW: f32 = 34.0;
const PITCH: f32 = ROW + 1.0;

fn table_id() -> widget::Id {
    widget::Id::new("table")
}

/// Scrolls the pane in the main area to `offset` pixels from the top.
/// The table, the words pane, and the import's failed table share one
/// widget id, since only one of them is in the window.
pub fn scroll_to(offset: f32) -> Task<Message> {
    widget::operation::scroll_to(table_id(), scrollable::AbsoluteOffset { x: 0.0, y: offset })
}

pub fn scroll_to_top() -> Task<Message> {
    scroll_to(0.0)
}

/// The table: the header row, then the rows in a scrollable column.
pub fn view<'a>(open: &'a Open, rows: Vec<&'a Book>) -> Element<'a, Message> {
    let mut headers = row![space().width(MARK)]
        .height(theme::HEADER)
        .align_y(Center);
    for column in COLUMNS {
        headers = headers.push(header(column, &open.query.sort));
    }
    frame(
        headers.into(),
        responsive(move |size| body(open, &rows, size)),
    )
}

/// A pane in the main area: the header row on the window ground, a line,
/// and the body on the surface ground, filling the area.
pub fn frame<'a>(
    headers: Element<'a, Message>,
    body: impl Into<Element<'a, Message>>,
) -> Element<'a, Message> {
    let pane = column![
        container(headers).style(theme::ground(|c| c.window)),
        theme::hline(),
        body.into(),
    ];
    container(pane)
        .width(Fill)
        .height(Fill)
        .style(theme::ground(|c| c.surface))
        .into()
}

/// The width of the File column of the failed table.
const FILE: f32 = 320.0;

/// The import's failed table: a File column and an Error column, one row
/// per file, at the book row height. The rows are not buttons: there is
/// no book to select.
pub fn failed_view<'a>(open: &'a Open, rows: Vec<(&'a Path, &'a str)>) -> Element<'a, Message> {
    let head = |name: &'static str, width: Length| {
        cell(
            theme::label(name).style(theme::text_color(|c| c.muted)),
            width,
        )
    };
    let headers = row![
        space().width(MARK),
        head("File", Length::Fixed(FILE)),
        head("Error", Fill),
    ]
    .height(theme::HEADER)
    .align_y(Center);
    let body = responsive(move |size| {
        self::rows(open.scroll, size, rows.len(), None, |i| failed_row(rows[i]))
    });
    frame(headers.into(), body)
}

fn failed_row<'a>((file, error): (&'a Path, &'a str)) -> Element<'a, Message> {
    let file = line(file_name(file)).font(MONO).size(12);
    let error = line(error).style(theme::text_color(|c| c.ink_2));
    let cells = row![
        space().width(MARK),
        cell(file, Length::Fixed(FILE)),
        cell(error, Fill),
    ]
    .height(Fill)
    .align_y(Center);
    column![container(cells).width(Fill).height(ROW), theme::hline()].into()
}

/// The last part of a path, or the whole path when it has none.
pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The table body: the rows the query selected, built through `rows`.
fn body<'a>(open: &'a Open, rows: &[&'a Book], size: Size) -> Element<'a, Message> {
    let selected = open.sidebar.as_ref().map(Sidebar::id);
    self::rows(open.scroll, size, rows.len(), None, |i| {
        book_row(rows[i], &open.progress, selected == Some(rows[i].id))
    })
}

/// A scrollable column of `len` rows at the row pitch, with only the rows
/// in view at `scroll` built by `build`. Blank space stands in for the
/// rows above and below them, so the scrollbar and the wheel behave as if
/// every row were there. Building every row would shape the text of
/// thousands of cells on each redraw.
///
/// `open` is the index of a row that `build` makes taller, and the extra
/// height. The words pane opens a row this way to show a definition
/// under it.
pub fn rows<'a>(
    scroll: f32,
    size: Size,
    len: usize,
    open: Option<(usize, f32)>,
    build: impl Fn(usize) -> Element<'a, Message>,
) -> Element<'a, Message> {
    let span = span(scroll, size.height, len, open);
    let built = column((span.first..span.last).map(build));
    let above = space().height(span.above);
    let below = space().height(span.below);
    scrollable(column![above, built, below])
        .id(table_id())
        .on_scroll(|viewport| Message::Scrolled(viewport.absolute_offset().y))
        .width(Fill)
        .height(Fill)
        .into()
}

/// The rows that `rows` builds, from `first` up to but not including
/// `last`, and the blank space above and below them in pixels.
#[derive(Debug, PartialEq)]
struct Span {
    first: usize,
    last: usize,
    above: f32,
    below: f32,
}

/// The rows in view at `scroll` in a view `height` pixels high, out of
/// `len` rows at the row pitch. `open` is the index of a row with extra
/// height, and that height. The rows under the open row move down by the
/// extra height.
///
/// The span holds one row more than fits, since the first row in view is
/// cut off at the top. At the end of the list it holds the last rows, so
/// no blank space shows under them.
fn span(scroll: f32, height: f32, len: usize, open: Option<(usize, f32)>) -> Span {
    let in_view = (height / PITCH).ceil() as usize + 1;
    let first = match open {
        // The view starts under the open row and its extra height.
        Some((row, extra)) if scroll >= (row + 1) as f32 * PITCH + extra => {
            ((scroll - extra) / PITCH) as usize
        }
        // The view starts at the open row or above it.
        Some((row, _)) => ((scroll / PITCH) as usize).min(row),
        None => (scroll / PITCH) as usize,
    };
    let first = first.min(len.saturating_sub(in_view));
    let last = (first + in_view).min(len);
    let (above, below) = match open {
        Some((row, extra)) if row < first => (extra, 0.0),
        Some((row, extra)) if row >= last => (0.0, extra),
        _ => (0.0, 0.0),
    };
    Span {
        first,
        last,
        above: first as f32 * PITCH + above,
        below: (len - last) as f32 * PITCH + below,
    }
}

/// A column header: the name, and on the sorted column an arrow for the
/// direction and a 2 px `accent` mark along the bottom edge, the same
/// mark the selected row wears on its left edge. The arrows are glyphs
/// of the interface typeface, so they sit on the label's baseline.
///
/// The button gets the column's width itself. A button takes a plain fill
/// from its content, not the fill portion, and the text headers would
/// come out equal widths.
fn header<'a>(column: SortKey, sort: &Sort) -> Element<'a, Message> {
    let sorted = sort.keys.first() == Some(&column);
    let mut label = row![theme::label(name(column))].spacing(4).align_y(Center);
    if sorted {
        let arrow = if sort.descending { "↓" } else { "↑" };
        label = label.push(theme::label(arrow).style(theme::text_color(|c| c.accent)));
    }
    let mark = container(space()).width(Fill).height(2);
    let mark = if sorted {
        mark.style(theme::ground(|c| c.accent))
    } else {
        mark
    };
    button(column![column_cell(label, column), mark])
        .on_press(Message::Sort(column))
        .width(width(column))
        .height(Fill)
        .padding(0)
        .style(theme::header(sorted))
        .into()
}

/// One row: a button with a cell per column and a 1 px line under it.
fn book_row<'a>(
    book: &'a Book,
    progress: &'a BTreeMap<i64, Vec<ProgressRow>>,
    selected: bool,
) -> Element<'a, Message> {
    let m = &book.metadata;
    let latest = query::latest(progress, book.id);

    let mark = container(space()).width(MARK).height(Fill);
    let mark = if selected {
        mark.style(theme::ground(|c| c.accent))
    } else {
        mark
    };
    let id = line(book.id)
        .font(MONO)
        .size(12)
        .style(theme::text_color(|c| c.muted));
    let title = line(&m.title).font(SANS_MEDIUM);
    let author = line(format::authors(&m.authors)).style(theme::text_color(|c| c.muted));
    let series: Element<'a, Message> = match &m.series {
        Some(s) => {
            let name = line(&s.name).style(theme::text_color(|c| c.muted));
            let mut parts = row![name].spacing(4);
            if let Some(n) = s.number {
                let number = format!("#{}", epubsync_core::metadata::format_series_number(n));
                parts = parts.push(line(number).style(theme::text_color(|c| c.faint)));
            }
            parts.into()
        }
        None => space().into(),
    };
    let words = line(
        book.stats
            .word_count
            .map(format::thousands)
            .unwrap_or_default(),
    )
    .style(theme::text_color(|c| c.muted));
    let ease = line(
        book.stats
            .reading_ease
            .map(|s| format!("{s:.0}"))
            .unwrap_or_default(),
    )
    .style(theme::text_color(|c| c.muted));
    let last_read = line(
        latest
            .and_then(ProgressRow::day)
            .map(format::day)
            .unwrap_or_default(),
    )
    .style(theme::text_color(|c| c.muted));
    let finished = line(
        query::finished(progress, book.id)
            .map(format::day)
            .unwrap_or_default(),
    )
    .style(theme::text_color(|c| c.muted));

    let cells = row![
        mark,
        column_cell(id, SortKey::Id),
        column_cell(title, SortKey::Title),
        column_cell(author, SortKey::Author),
        column_cell(series, SortKey::Series),
        column_cell(words, SortKey::Words),
        column_cell(ease, SortKey::Ease),
        column_cell(progress_cell(latest), SortKey::Progress),
        column_cell(last_read, SortKey::LastRead),
        column_cell(finished, SortKey::Finished),
    ]
    .height(Fill)
    .align_y(Center);
    column![
        button(cells)
            .on_press(Message::Select(book.id))
            .width(Fill)
            .height(ROW)
            .padding(0)
            .style(theme::row(selected)),
        theme::hline(),
    ]
    .into()
}

/// Cell text at the body size on one line. The cell clips what does not
/// fit.
pub fn line<'a>(content: impl text::IntoFragment<'a>) -> Text<'a> {
    text(content).size(BODY).wrapping(text::Wrapping::None)
}

/// A cell: the given width, the row's full height with the content
/// centered in it, 12 px side padding, one line, clipped.
pub fn cell<'a>(
    content: impl Into<Element<'a, Message>>,
    width: Length,
) -> widget::Container<'a, Message> {
    container(content)
        .width(width)
        .height(Fill)
        .align_y(Center)
        .padding(padding::horizontal(12))
        .clip(true)
}

/// A cell in one of the table's columns. The number columns are
/// right-aligned.
fn column_cell<'a>(
    content: impl Into<Element<'a, Message>>,
    column: SortKey,
) -> Element<'a, Message> {
    let mut cell = cell(content, width(column));
    if matches!(column, SortKey::Id | SortKey::Words | SortKey::Ease) {
        cell = cell.align_x(Right);
    }
    cell.into()
}

/// The latest progress. A book that is being read shows a bar and the
/// percent. A finished book shows the word READ. A book with no progress
/// row, or one that is not started, shows nothing.
fn progress_cell<'a>(latest: Option<&'a ProgressRow>) -> Element<'a, Message> {
    match latest {
        Some(p) if p.status == ReadStatus::Finished => text("READ")
            .size(11)
            .font(SANS_MEDIUM)
            .style(theme::text_color(|c| c.finished))
            .into(),
        Some(p) if p.status == ReadStatus::Reading => row![
            progress_bar(0.0..=100.0, p.percent as f32)
                .length(64)
                .girth(4)
                .style(theme::bar(p.status)),
            line(format!("{}%", p.percent))
                .width(34)
                .style(theme::text_color(|c| c.ink_2)),
        ]
        .spacing(6)
        .align_y(Center)
        .into(),
        _ => space().into(),
    }
}

/// The status word in upper case on its tint.
pub fn chip<'a>(status: ReadStatus) -> Element<'a, Message> {
    container(
        text(format::status(status).to_uppercase())
            .size(11)
            .font(SANS_MEDIUM),
    )
    .padding([1, 6])
    .style(theme::chip(status))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    // A view 350 px high holds 10 rows at the pitch of 35 px, and the
    // span holds 11.
    const HEIGHT: f32 = 350.0;
    const PANEL: f32 = 280.0;

    fn pitch(rows: usize) -> f32 {
        rows as f32 * PITCH
    }

    #[test]
    fn with_no_open_row_the_span_starts_at_the_scroll_offset() {
        let span = span(pitch(4) + 10.0, HEIGHT, 100, None);
        assert_eq!(
            span,
            Span {
                first: 4,
                last: 15,
                above: pitch(4),
                below: pitch(85),
            }
        );
    }

    #[test]
    fn an_open_row_above_the_view_adds_its_height_above() {
        let span = span(pitch(10) + PANEL, HEIGHT, 100, Some((2, PANEL)));
        assert_eq!(
            span,
            Span {
                first: 10,
                last: 21,
                above: pitch(10) + PANEL,
                below: pitch(79),
            }
        );
    }

    #[test]
    fn an_open_row_in_the_view_is_built_with_the_rows() {
        let span = span(pitch(3), HEIGHT, 100, Some((5, PANEL)));
        assert_eq!(
            span,
            Span {
                first: 3,
                last: 14,
                above: pitch(3),
                below: pitch(86),
            }
        );
        // A view that starts inside the panel starts at the open row.
        let span = self::span(pitch(6) + 100.0, HEIGHT, 100, Some((5, PANEL)));
        assert_eq!(span.first, 5);
        assert_eq!(span.above, pitch(5));
    }

    #[test]
    fn an_open_row_below_the_view_adds_its_height_below() {
        let span = span(0.0, HEIGHT, 100, Some((50, PANEL)));
        assert_eq!(
            span,
            Span {
                first: 0,
                last: 11,
                above: 0.0,
                below: pitch(89) + PANEL,
            }
        );
    }

    #[test]
    fn the_rows_and_the_space_add_up_to_the_full_height() {
        for scroll in [0.0, 500.0, 1000.0, 3000.0, 3600.0] {
            let span = span(scroll, HEIGHT, 100, Some((20, PANEL)));
            let built = pitch(span.last - span.first)
                + if (span.first..span.last).contains(&20) {
                    PANEL
                } else {
                    0.0
                };
            assert_eq!(span.above + built + span.below, pitch(100) + PANEL);
        }
    }
}
