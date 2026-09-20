//! The sidebar: the selected book's details, the Open and Remove…
//! buttons, its description, its progress on each device, and the words
//! looked up in it.

use epubsync_core::device::ReadStatus;
use epubsync_core::library::{Book, Field, ProgressRow, WordRow, book_file_name};
use epubsync_core::metadata::format_series_number;
use iced::widget::{
    button, column, container, image, markdown, progress_bar, row, scrollable, space, text,
};
use iced::{Center, Color, ContentFit, Element, Fill, padding};

use crate::theme::{self, MONO, SANS_MEDIUM, SERIF, SERIF_MEDIUM};
use crate::{Message, Open, Selected, format, table};

const WIDTH: f32 = 360.0;
/// The side padding of the header, the body, and the footer.
const INSET: f32 = 20.0;
/// The height of the cover at the top of the body.
const COVER_HEIGHT: f32 = 220.0;

/// The link color in the description. The markdown widget takes its
/// colors before it is drawn, when the mode is not known, so this one
/// color reads on both grounds.
const LINK: Color = Color::from_rgb8(0x4A, 0x8F, 0xA0);

/// The sidebar: a header, a scrollable body, and a footer, with a 1 px
/// line on its left.
pub fn view<'a>(open: &'a Open, book: &'a Book, selected: &'a Selected) -> Element<'a, Message> {
    let pane = column![
        header(book.id),
        theme::hline(),
        scrollable(body(open, book, selected)).height(Fill),
        theme::hline(),
        footer(open, book),
    ];
    row![
        theme::vline(),
        container(pane)
            .width(WIDTH)
            .height(Fill)
            .style(theme::ground(|c| c.window)),
    ]
    .into()
}

/// "Book 13" and the close button. The label starts at the body's left
/// inset, and the close button's glyph ends at the body's right inset.
fn header<'a>(id: i64) -> Element<'a, Message> {
    let close = button(container(text("×").size(15)).center(22))
        .on_press(Message::Close)
        .padding(0)
        .style(theme::close);
    row![
        theme::label(format!("Book {id}")).style(theme::text_color(|c| c.muted)),
        space().width(Fill),
        close,
    ]
    .align_y(Center)
    .height(theme::HEADER)
    .padding(padding::left(INSET).right(INSET - 6.0))
    .into()
}

fn body<'a>(open: &'a Open, book: &'a Book, selected: &'a Selected) -> Element<'a, Message> {
    let m = &book.metadata;

    let title = text(&m.title).size(26).font(SERIF_MEDIUM).line_height(1.15);
    let mut byline = row![
        text(format::authors(&m.authors))
            .size(14)
            .style(theme::text_color(|c| c.ink_2))
    ]
    .spacing(4);
    if let Some(series) = &m.series {
        byline = byline.push(
            text(format!("· {}", format::series_tag(series)))
                .size(14)
                .style(theme::text_color(|c| c.muted)),
        );
    }

    let open_button = button(text("Open").size(12))
        .on_press(Message::OpenBook)
        .padding([4, 9])
        .style(theme::action);
    let remove = button(text("Remove…").size(12))
        .on_press_maybe(open.library.is_some().then_some(Message::AskRemove))
        .padding([4, 9])
        .style(theme::danger);
    let actions = row![open_button, remove].spacing(8);

    let mut meta = column![].spacing(6);
    for f in book.fields() {
        let (label, value) = match f {
            Field::Publisher(p) => ("Publisher", p.to_string()),
            Field::Series(s) => match s.number {
                Some(n) => (
                    "Series",
                    format!("{}, book {}", s.name, format_series_number(n)),
                ),
                None => ("Series", s.name.clone()),
            },
            Field::WordCount(w) => ("Length", format!("{} words", format::thousands(w))),
            Field::ReadingEase(e) => ("Ease", format::reading_ease(e)),
        };
        meta = meta.push(field(label, text(value).size(12.5)));
    }
    meta = meta.push(field(
        "File",
        text(book_file_name(book.id)).font(MONO).size(11.5),
    ));

    let description: Element<'a, Message> = if selected.description.is_empty() {
        text("No description in the file.")
            .size(13)
            .style(theme::text_color(|c| c.faint))
            .into()
    } else {
        markdown::view(&selected.description, description_settings()).map(Message::OpenLink)
    };

    let mut devices = column![heading("ON DEVICE")].spacing(8);
    let rows = open
        .progress
        .get(&book.id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    if rows.is_empty() {
        devices = devices.push(
            text("Not yet sent to a device.")
                .size(12.5)
                .style(theme::text_color(|c| c.faint)),
        );
    }
    for (i, p) in rows.iter().enumerate() {
        if i > 0 {
            devices = devices.push(theme::hline());
        }
        devices = devices.push(device(p));
    }

    let looked_up: Vec<&WordRow> = open.words.iter().filter(|w| w.book_id == book.id).collect();
    let mut words = column![heading("WORDS")].spacing(8);
    if looked_up.is_empty() {
        words = words.push(
            text("No words looked up in this book.")
                .size(12.5)
                .style(theme::text_color(|c| c.faint)),
        );
    } else {
        words = words.push(column(looked_up.into_iter().map(word)).spacing(4));
    }

    column![]
        .extend(cover(open, book.id))
        .push(column![title, byline.wrap()].spacing(6))
        .push(actions)
        .push(theme::hline())
        .push(meta)
        .push(theme::hline())
        .push(description)
        .push(theme::hline())
        .push(devices)
        .push(theme::hline())
        .push(words)
        .spacing(14)
        .padding(padding::top(18).bottom(24).left(INSET).right(INSET))
        .into()
}

/// The cover above the title, centered and `COVER_HEIGHT` tall, with the
/// aspect ratio kept. A book the library has no cover for gets the faint
/// words "No cover" instead. A book the library has not read yet gets
/// nothing, and the place above the title stays empty.
fn cover<'a>(open: &'a Open, id: i64) -> Option<Element<'a, Message>> {
    let block: Element<'a, Message> = match open.covers.get(&id)? {
        Some(handle) => image(handle)
            .height(COVER_HEIGHT)
            .content_fit(ContentFit::Contain)
            .into(),
        None => text("No cover")
            .size(12.5)
            .style(theme::text_color(|c| c.faint))
            .into(),
    };
    Some(container(block).center_x(Fill).into())
}

/// A block heading in upper case: "ON DEVICE", "WORDS".
fn heading<'a>(label: &'a str) -> Element<'a, Message> {
    text(label)
        .size(11.5)
        .font(SANS_MEDIUM)
        .style(theme::text_color(|c| c.muted))
        .into()
}

/// One looked-up word and the day it was looked up.
fn word(w: &WordRow) -> Element<'_, Message> {
    row![
        text(&w.word).size(13.5),
        space().width(Fill),
        text(format::day(w.day()))
            .size(12.5)
            .style(theme::text_color(|c| c.muted)),
    ]
    .spacing(12)
    .align_y(Center)
    .into()
}

/// A label and its value on one line.
fn field<'a>(label: &'a str, value: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    row![
        text(label)
            .size(12.5)
            .width(72)
            .style(theme::text_color(|c| c.muted)),
        value.into(),
    ]
    .spacing(14)
    .into()
}

/// One device's progress: the serial and the day, a full-width bar, the
/// percent and the status chip, then the reading time when the device
/// counted any. A finished book's day is the day it was finished; any
/// other book's is the day it was last read.
fn device(p: &ProgressRow) -> Element<'_, Message> {
    let when = match (p.status, p.finished_day(), p.day()) {
        (ReadStatus::Finished, Some(day), _) => format!("finished {}", format::day(day)),
        (_, _, Some(day)) => format!("read {}", format::day(day)),
        (_, _, None) => "not opened".to_string(),
    };
    let mut block = column![
        row![
            text(&p.device_serial)
                .font(MONO)
                .size(11.5)
                .style(theme::text_color(|c| c.ink_2)),
            space().width(Fill),
            text(when).size(12.5).style(theme::text_color(|c| c.muted)),
        ]
        .align_y(Center),
        progress_bar(0.0..=100.0, p.percent as f32)
            .length(Fill)
            .girth(5)
            .style(theme::bar(p.status)),
        row![
            text(format!("{}%", p.percent))
                .size(12.5)
                .style(theme::text_color(|c| c.ink_2)),
            space().width(Fill),
            table::chip(p.status),
        ]
        .align_y(Center),
    ]
    .spacing(6);
    if let Some(seconds) = p.time_spent.filter(|s| *s > 0) {
        block = block.push(
            text(format!("{} of reading", format::duration(seconds)))
                .size(12.5)
                .style(theme::text_color(|c| c.muted)),
        );
    }
    block.into()
}

/// The description in the serif face at 15.5 px.
fn description_settings() -> markdown::Settings {
    let style = markdown::Style {
        font: SERIF,
        link_color: LINK,
        ..markdown::Style::from_palette(iced::theme::Palette::LIGHT)
    };
    markdown::Settings::with_text_size(15.5, style)
}

/// The full file path on one line, clipped.
fn footer<'a>(open: &'a Open, book: &'a Book) -> Element<'a, Message> {
    let path = open.folder.join(book_file_name(book.id));
    container(
        text(path.display().to_string())
            .font(MONO)
            .size(11)
            .wrapping(text::Wrapping::None)
            .style(theme::text_color(|c| c.muted)),
    )
    .width(Fill)
    .clip(true)
    .padding([9.0, INSET])
    .into()
}
