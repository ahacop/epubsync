//! The words pane: every word looked up on a device, newest first, with
//! the book it came from, the device, and the day. It stands in for the
//! table while the Words tab is selected. A click on a row opens the
//! Webster's 1913 entry for the word in a panel under the row, and a
//! second click closes it.

use epubsync_core::library::WordRow;
use iced::advanced::graphics::text::Paragraph;
use iced::advanced::text::{Alignment, LineHeight, Paragraph as _, Shaping, Text, Wrapping};
use iced::alignment::Vertical;
use iced::widget::{button, column, container, responsive, row, scrollable, text, text_input};
use iced::{Center, Element, Fill, Length, Size};

use crate::theme::{self, BODY, MONO, SANS_MEDIUM};
use crate::{Message, Open, format, table};

/// The columns from left to right.
#[derive(Debug, Clone, Copy)]
enum Column {
    Word,
    Book,
    Device,
    Day,
}

const COLUMNS: [Column; 4] = [Column::Word, Column::Book, Column::Device, Column::Day];

fn name(column: Column) -> &'static str {
    match column {
        Column::Word => "Word",
        Column::Book => "Book",
        Column::Device => "Device",
        Column::Day => "Looked up",
    }
}

/// The fixed columns take pixels; the text columns share the rest.
fn width(column: Column) -> Length {
    match column {
        Column::Word => Length::FillPortion(30),
        Column::Book => Length::FillPortion(50),
        Column::Device => Length::Fixed(136.0),
        Column::Day => Length::Fixed(108.0),
    }
}

/// The words that match the filter text, in the order given. The match is
/// a case-insensitive substring of the word or of the book title.
pub fn select<'a>(words: &'a [WordRow], filter: &str) -> Vec<&'a WordRow> {
    let needle = filter.trim().to_lowercase();
    words
        .iter()
        .filter(|w| {
            needle.is_empty()
                || w.word.to_lowercase().contains(&needle)
                || w.book_title.to_lowercase().contains(&needle)
        })
        .collect()
}

/// The pane: the header row, then the rows in a scrollable column. A
/// library with no words at all shows a note in place of the rows.
pub fn view<'a>(open: &'a Open, rows: Vec<&'a WordRow>) -> Element<'a, Message> {
    let mut headers = row![].height(theme::HEADER).align_y(Center);
    for column in COLUMNS {
        let label = theme::label(name(column)).style(theme::text_color(|c| c.muted));
        headers = headers.push(table::cell(label, width(column)));
    }
    let body: Element<'a, Message> = if open.words.is_empty() {
        container(
            text("No words yet. Sync reads the words looked up on the Kobo.")
                .size(13)
                .style(theme::text_color(|c| c.faint)),
        )
        .padding(20)
        .into()
    } else {
        responsive(move |size| body(open, &rows, size)).into()
    };
    table::frame(headers.into(), body)
}

fn body<'a>(open: &'a Open, rows: &[&'a WordRow], size: Size) -> Element<'a, Message> {
    let definition = open
        .definition
        .as_ref()
        .and_then(|(id, entries)| Some((rows.iter().position(|w| w.id == *id)?, entries)));
    let panel = definition.map(|(i, _)| (i, PANEL));
    table::rows(open.scroll, size, rows.len(), panel, |i| {
        let entries = definition.and_then(|(open, entries)| (open == i).then_some(entries));
        word_row(rows[i], entries)
    })
}

/// One row: a button with a cell per column and a 1 px line under it.
/// The open row wears the selected tint and has the definition panel
/// between the button and the line.
fn word_row<'a>(
    w: &'a WordRow,
    entries: Option<&'a Result<Vec<String>, String>>,
) -> Element<'a, Message> {
    let book = table::line(&w.book_title).style(theme::text_color(|c| c.muted));
    let device = table::line(&w.device_serial)
        .font(MONO)
        .size(12)
        .style(theme::text_color(|c| c.muted));
    let day = table::line(format::day(w.day())).style(theme::text_color(|c| c.muted));
    let cells = row![
        table::cell(word(&w.word), width(Column::Word)),
        table::cell(book, width(Column::Book)),
        table::cell(device, width(Column::Device)),
        table::cell(day, width(Column::Day)),
    ]
    .height(Fill)
    .align_y(Center);
    let mut parts = column![
        button(cells)
            .on_press(Message::Define(w.id))
            .width(Fill)
            .height(table::ROW)
            .padding(0)
            .style(theme::row(entries.is_some())),
    ];
    if let Some(entries) = entries {
        parts = parts.push(panel(entries));
    }
    parts.push(theme::hline()).into()
}

/// The word as a text input that takes no input, so a reader can select
/// the word and copy it. The input captures a click, so it gets the width
/// of the word and no more: a click on the rest of the cell reaches the
/// row button and opens the panel.
fn word(word: &str) -> Element<'_, Message> {
    let content = Text {
        content: word,
        bounds: Size::INFINITE,
        size: BODY.into(),
        line_height: LineHeight::default(),
        font: SANS_MEDIUM,
        align_x: Alignment::Default,
        align_y: Vertical::Center,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
    };
    // One pixel more than the text, so the input never scrolls its text.
    let width = Paragraph::with_text(content).min_width().ceil() + 1.0;
    text_input("", word)
        .width(width)
        .padding(0)
        .font(SANS_MEDIUM)
        .size(BODY)
        .style(theme::word)
        .into()
}

/// The height of the definition panel. A fixed height keeps the row
/// arithmetic of `table::rows` exact, and the entries scroll inside it.
const PANEL: f32 = 280.0;

/// The definition panel: each entry in the monospace face, with its line
/// breaks, and a line between two entries. A word the dictionary has no
/// entry for gets a sentence that says so.
fn panel(entries: &Result<Vec<String>, String>) -> Element<'_, Message> {
    let note = |s: String| {
        text(s)
            .size(13)
            .style(theme::text_color(|c| c.faint))
            .into()
    };
    let body: Element<'_, Message> = match entries {
        Ok(entries) if entries.is_empty() => note("No entry in Webster's 1913.".into()),
        Ok(entries) => {
            let mut body = column![].spacing(12);
            for (i, entry) in entries.iter().enumerate() {
                if i > 0 {
                    body = body.push(theme::hline());
                }
                body = body.push(text(entry).font(MONO).size(12));
            }
            body.into()
        }
        Err(e) => note(format!("Could not read the dictionary: {e}")),
    };
    container(scrollable(container(body).width(Fill).padding([12, 12])).height(Fill))
        .width(Fill)
        .height(PANEL)
        .style(theme::ground(|c| c.window))
        .into()
}

/// Whether the Kobo looked the word up in its English dictionary, which
/// it records as "-en". A row with no dictionary counts as English. The
/// panel looks up only English words.
pub fn english(w: &WordRow) -> bool {
    matches!(w.dict_suffix.as_deref(), None | Some("" | "-en"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word(word: &str, book_id: i64, book_title: &str) -> WordRow {
        WordRow {
            id: book_id,
            word: word.into(),
            device_serial: "N123".into(),
            book_id,
            book_title: book_title.into(),
            dict_suffix: None,
            looked_up_at: "2026-09-08T10:00:00Z".into(),
        }
    }

    #[test]
    fn only_an_english_dictionary_counts_as_english() {
        let with = |suffix: Option<&str>| WordRow {
            dict_suffix: suffix.map(String::from),
            ..word("vex", 1, "Emma")
        };
        assert!(english(&with(Some("-en"))));
        assert!(english(&with(None)));
        assert!(english(&with(Some(""))));
        assert!(!english(&with(Some("-fr"))));
    }

    #[test]
    fn an_empty_filter_keeps_every_word_in_order() {
        let words = [word("vex", 1, "Emma"), word("hale", 2, "Persuasion")];
        let picked: Vec<&str> = select(&words, "  ")
            .iter()
            .map(|w| w.word.as_str())
            .collect();
        assert_eq!(picked, ["vex", "hale"]);
    }

    #[test]
    fn the_filter_matches_the_word_or_the_title() {
        let words = [
            word("vex", 1, "Emma"),
            word("hale", 2, "Persuasion"),
            word("emmanuel", 3, "Villette"),
        ];
        let pick =
            |f: &str| -> Vec<&str> { select(&words, f).iter().map(|w| w.word.as_str()).collect() };
        assert_eq!(pick("EMM"), ["vex", "emmanuel"]);
        assert_eq!(pick("persua"), ["hale"]);
        assert_eq!(pick("zzz"), Vec::<&str>::new());
    }
}
