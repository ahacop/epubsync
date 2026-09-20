//! The edit form in the sidebar: one input per field the app owns, with
//! Save and Cancel under them. `Form::new` fills the inputs from a
//! book, and `Form::record` reads them back as the `Metadata` record
//! that `Library::edit` writes.
//!
//! `Form::record` follows the rules the CLI's `edit` command follows.
//! Each text is trimmed, and an empty text clears its field: no
//! publisher, no series, no description. An empty title is an error,
//! because every book needs a title. An author row with an empty name
//! is dropped. An author with an empty sort name gets the sort name
//! that `sort_name` makes from the display name. An empty series name
//! clears the series and the form ignores the number beside it. A
//! number that is not a number is an error.
//!
//! The description is the one field the form does not trim. It goes
//! into the record as the reader typed it, HTML included, so the markup
//! a publisher wrote survives an edit of the other fields. A
//! description of only whitespace clears the field.

use epubsync_core::library::Book;
use epubsync_core::metadata::{Author, Metadata, Series, format_series_number};
use epubsync_core::sort_name::sort_name;
use iced::widget::{button, column, container, row, space, text, text_editor, text_input};
use iced::{Center, Element, Fill, padding};

use crate::theme::{self, SANS, SANS_MEDIUM};
use crate::{Message, Open, detail};

/// The height of the description editor.
const EDITOR_HEIGHT: f32 = 160.0;
/// The width of the series number input.
const NUMBER_WIDTH: f32 = 64.0;

/// The edit form: the text of each input, as the reader has typed it.
pub struct Form {
    pub id: i64,
    pub title: String,
    pub authors: Vec<AuthorField>,
    pub series: String,
    pub series_number: String,
    pub publisher: String,
    pub description: text_editor::Content,
    /// Why the last Save did not save, shown under the buttons.
    pub error: Option<String>,
}

/// One author row: the display name and the sort name beside it.
pub struct AuthorField {
    pub name: String,
    pub sort: String,
}

/// A change to one input of the form.
#[derive(Debug, Clone)]
pub enum Change {
    Title(String),
    AuthorName(usize, String),
    AuthorSort(usize, String),
    AddAuthor,
    RemoveAuthor(usize),
    Series(String),
    SeriesNumber(String),
    Publisher(String),
    Description(text_editor::Action),
}

impl Form {
    /// The form for a book, with each input filled from its record. A
    /// field the record does not hold gives an empty input.
    pub fn new(book: &Book) -> Form {
        let m = &book.metadata;
        Form {
            id: book.id,
            title: m.title.clone(),
            authors: m
                .authors
                .iter()
                .map(|a| AuthorField {
                    name: a.name.clone(),
                    sort: a.sort.clone(),
                })
                .collect(),
            series: m
                .series
                .as_ref()
                .map(|s| s.name.clone())
                .unwrap_or_default(),
            series_number: m
                .series
                .as_ref()
                .and_then(|s| s.number)
                .map(format_series_number)
                .unwrap_or_default(),
            publisher: m.publisher.clone().unwrap_or_default(),
            description: text_editor::Content::with_text(m.description.as_deref().unwrap_or("")),
            error: None,
        }
    }

    /// Puts the text of one input into the form. It also clears the
    /// error, so a corrected input does not sit under a stale message.
    pub fn apply(&mut self, change: Change) {
        self.error = None;
        match change {
            Change::Title(t) => self.title = t,
            Change::AuthorName(i, t) => {
                if let Some(a) = self.authors.get_mut(i) {
                    a.name = t;
                }
            }
            Change::AuthorSort(i, t) => {
                if let Some(a) = self.authors.get_mut(i) {
                    a.sort = t;
                }
            }
            Change::AddAuthor => self.authors.push(AuthorField {
                name: String::new(),
                sort: String::new(),
            }),
            Change::RemoveAuthor(i) => {
                if i < self.authors.len() {
                    self.authors.remove(i);
                }
            }
            Change::Series(t) => self.series = t,
            Change::SeriesNumber(t) => self.series_number = t,
            Change::Publisher(t) => self.publisher = t,
            Change::Description(action) => self.description.perform(action),
        }
    }

    /// The record the inputs make, or one sentence that says why they
    /// make none. The module doc lists the rules.
    pub fn record(&self) -> Result<Metadata, String> {
        let title = self.title.trim();
        if title.is_empty() {
            return Err("The title is empty.".to_string());
        }
        let authors = self
            .authors
            .iter()
            .filter_map(|a| {
                let name = a.name.trim();
                if name.is_empty() {
                    return None;
                }
                let sort = a.sort.trim();
                Some(Author {
                    name: name.to_string(),
                    sort: if sort.is_empty() {
                        sort_name(name)
                    } else {
                        sort.to_string()
                    },
                })
            })
            .collect();
        let name = self.series.trim();
        let series = if name.is_empty() {
            None
        } else {
            let number = match self.series_number.trim() {
                "" => None,
                n => Some(
                    n.parse::<f64>()
                        .map_err(|_| "The series number is not a number.".to_string())?,
                ),
            };
            Some(Series {
                name: name.to_string(),
                number,
            })
        };
        Ok(Metadata {
            title: title.to_string(),
            authors,
            series,
            publisher: Some(self.publisher.trim().to_string()).filter(|p| !p.is_empty()),
            description: Some(self.description.text()).filter(|d| !d.trim().is_empty()),
        })
    }
}

/// The form in the sidebar body: the cover, one row per field, the
/// Cancel and Save buttons, and the error line of a Save that did not
/// save. The description reads as the plain text of the file, not as
/// markdown, and the device and word blocks come back with the read
/// view.
pub fn body<'a>(open: &'a Open, book: &'a Book, form: &'a Form) -> Element<'a, Message> {
    column![]
        .extend(detail::cover(open, book.id))
        .push(detail::field(
            "Title",
            input("Title", &form.title, Change::Title),
        ))
        .push(detail::field("Authors", authors(form)))
        .push(detail::field("Series", series(form)))
        .push(detail::field(
            "Publisher",
            input("Publisher", &form.publisher, Change::Publisher),
        ))
        .push(detail::field("Description", description(form)))
        .push(buttons(open.library.is_some()))
        .extend(form.error.as_deref().map(error_line))
        .spacing(14)
        .padding(
            padding::top(18)
                .bottom(24)
                .left(detail::INSET)
                .right(detail::INSET),
        )
        .into()
}

/// One single-line input in the filter field's shape. Enter in it saves
/// the form.
fn input<'a>(
    placeholder: &str,
    value: &str,
    change: impl Fn(String) -> Change + 'a,
) -> Element<'a, Message> {
    text_input(placeholder, value)
        .on_input(move |t| Message::Form(change(t)))
        .on_submit(Message::Save)
        .size(13)
        .padding([5, 10])
        .style(theme::filter)
        .into()
}

/// One row per author: the display name, the sort name, and an × that
/// drops the row. The sort input's placeholder is the sort name the
/// record gets when the input stays empty, so the reader sees it before
/// the save. An "Add author" button sits under the rows.
fn authors(form: &Form) -> Element<'_, Message> {
    let mut rows = column![].spacing(6);
    for (i, a) in form.authors.iter().enumerate() {
        let name = input("Name", &a.name, move |t| Change::AuthorName(i, t));
        let sort = text_input(&sort_name(&a.name), &a.sort)
            .on_input(move |t| Message::Form(Change::AuthorSort(i, t)))
            .on_submit(Message::Save)
            .size(13)
            .padding([5, 10])
            .style(theme::filter);
        let drop = button(container(text("×").size(15)).center(22))
            .on_press(Message::Form(Change::RemoveAuthor(i)))
            .padding(0)
            .style(theme::close);
        rows = rows.push(row![name, sort, drop].spacing(6).align_y(Center));
    }
    let add = button(text("Add author").size(12))
        .on_press(Message::Form(Change::AddAuthor))
        .padding([4, 9])
        .style(theme::action);
    rows.push(add).into()
}

/// The series name and its number on one row.
fn series(form: &Form) -> Element<'_, Message> {
    let number = text_input("#", &form.series_number)
        .on_input(|t| Message::Form(Change::SeriesNumber(t)))
        .on_submit(Message::Save)
        .width(NUMBER_WIDTH)
        .size(13)
        .padding([5, 10])
        .style(theme::filter);
    row![
        input("Series", &form.series, Change::Series),
        number.width(NUMBER_WIDTH),
    ]
    .spacing(6)
    .align_y(Center)
    .into()
}

/// The description editor. Enter inserts a line break here, so the
/// buttons are the only way to save from it.
fn description(form: &Form) -> Element<'_, Message> {
    text_editor(&form.description)
        .on_action(|a| Message::Form(Change::Description(a)))
        .min_height(EDITOR_HEIGHT)
        .font(SANS)
        .size(13)
        .padding([5, 10])
        .style(theme::editor)
        .into()
}

/// Cancel and Save on the right. Save is off while the import task
/// holds the library.
fn buttons<'a>(can_save: bool) -> Element<'a, Message> {
    let cancel = button(text("Cancel").size(12))
        .on_press(Message::CancelEdit)
        .padding([4, 9])
        .style(theme::action);
    let save = button(text("Save").size(12).font(SANS_MEDIUM))
        .on_press_maybe(can_save.then_some(Message::Save))
        .padding([4, 9])
        .style(theme::primary);
    row![space().width(Fill), cancel, save].spacing(8).into()
}

/// Why the last Save did not save.
fn error_line(why: &str) -> Element<'_, Message> {
    text(why)
        .size(12.5)
        .style(theme::text_color(|c| c.danger))
        .into()
}

#[cfg(test)]
mod tests {
    use epubsync_core::metadata::Stats;

    use super::*;

    /// A form with a title and every other input empty.
    fn form() -> Form {
        Form {
            id: 1,
            title: "A Book".to_string(),
            authors: Vec::new(),
            series: String::new(),
            series_number: String::new(),
            publisher: String::new(),
            description: text_editor::Content::new(),
            error: None,
        }
    }

    fn author(name: &str, sort: &str) -> AuthorField {
        AuthorField {
            name: name.to_string(),
            sort: sort.to_string(),
        }
    }

    #[test]
    fn an_empty_title_is_an_error() {
        let mut form = form();
        form.title = "  ".to_string();
        assert_eq!(form.record(), Err("The title is empty.".to_string()));
    }

    #[test]
    fn an_empty_sort_name_is_made_from_the_display_name() {
        let mut form = form();
        form.authors = vec![
            author("Ursula K. Le Guin", " "),
            author("Ann Leckie", "L, A"),
        ];
        let authors = form.record().unwrap().authors;
        assert_eq!(authors[0].sort, "Le Guin, Ursula K.");
        assert_eq!(authors[1].sort, "L, A");
    }

    #[test]
    fn an_author_row_with_an_empty_name_is_dropped() {
        let mut form = form();
        form.authors = vec![author("Ann Leckie", ""), author("  ", "Nobody")];
        let authors = form.record().unwrap().authors;
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "Ann Leckie");
    }

    #[test]
    fn an_empty_series_name_clears_the_series_and_its_number() {
        let mut form = form();
        form.series_number = "3".to_string();
        assert_eq!(form.record().unwrap().series, None);
    }

    #[test]
    fn a_series_number_parses_or_is_an_error() {
        let mut form = form();
        form.series = "Imperial Radch".to_string();
        form.series_number = "2.5".to_string();
        assert_eq!(
            form.record().unwrap().series,
            Some(Series {
                name: "Imperial Radch".to_string(),
                number: Some(2.5),
            })
        );

        form.series_number = "two".to_string();
        assert_eq!(
            form.record(),
            Err("The series number is not a number.".to_string())
        );
    }

    #[test]
    fn an_empty_publisher_and_a_blank_description_clear_their_fields() {
        let mut form = form();
        form.publisher = "  ".to_string();
        form.description = text_editor::Content::with_text("  \n  ");
        let record = form.record().unwrap();
        assert_eq!(record.publisher, None);
        assert_eq!(record.description, None);
    }

    #[test]
    fn a_full_record_survives_a_trip_through_the_form() {
        let record = Metadata {
            title: "Ancillary Justice".to_string(),
            authors: vec![Author {
                name: "Ann Leckie".to_string(),
                sort: "Leckie, Ann".to_string(),
            }],
            series: Some(Series {
                name: "Imperial Radch".to_string(),
                number: Some(1.0),
            }),
            publisher: Some("Orbit".to_string()),
            description: Some("<p>A ship, a body.</p>".to_string()),
        };
        let book = Book {
            id: 1,
            revision: 0,
            metadata: record.clone(),
            stats: Stats::default(),
        };
        assert_eq!(Form::new(&book).record().unwrap(), record);
    }
}
