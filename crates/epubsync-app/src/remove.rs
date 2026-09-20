//! Remove from the viewer: the Remove… button in the sidebar and the
//! dialog it opens. The dialog names the book and says what the
//! removal does. Its Remove button runs `Library::remove`, which deletes
//! the file and marks the row, and the window reloads after.
//!
//! The dialog follows the desktop conventions for a step that deletes
//! something. The title is a question that names the action and the
//! book. The body says what happens, so the reader decides on facts
//! rather than on "Are you sure?". The buttons are verbs, not Yes and
//! No. Cancel sits left of Remove, so the button that acts is the last
//! one on the right, and it is red. Escape and a click on the dimmed
//! window cancel. Enter does nothing, so a stray key press removes no
//! book.

use epubsync_core::library::Book;
use iced::widget::{
    button, center, column, container, mouse_area, opaque, row, space, stack, text,
};
use iced::{Element, Fill};

use crate::Message;
use crate::theme::{self, BODY, SANS_MEDIUM, SANS_SEMIBOLD};

const WIDTH: f32 = 440.0;

/// What the removal does, for the dialog's body.
const EFFECT: &str = "The file is deleted from the library folder, and the next sync takes the \
    book off any device it was sent to. The reading history and the looked-up words stay.";

/// The dialog's title: "Remove “The Left Hand of Darkness”?".
fn title(book: &str) -> String {
    format!("Remove \u{201C}{book}\u{201D}?")
}

/// The window with the dialog over it. The window shows through a dim
/// layer that takes no clicks, except that a click on it cancels. The
/// Remove button is off while the import task holds the library.
pub fn over<'a>(
    window: Element<'a, Message>,
    book: &'a Book,
    can_remove: bool,
) -> Element<'a, Message> {
    let cancel = button(text("Cancel").size(13))
        .on_press(Message::CancelRemove)
        .padding([6, 14])
        .style(theme::action);
    let remove = button(text("Remove").size(13).font(SANS_MEDIUM))
        .on_press_maybe(can_remove.then_some(Message::ConfirmRemove))
        .padding([6, 14])
        .style(theme::danger);
    let dialog = container(
        column![
            text(title(&book.metadata.title))
                .size(17)
                .font(SANS_SEMIBOLD)
                .line_height(1.3),
            text(EFFECT)
                .size(BODY)
                .line_height(1.45)
                .style(theme::text_color(|c| c.ink_2)),
            row![space().width(Fill), cancel, remove].spacing(10),
        ]
        .spacing(14)
        .padding(22),
    )
    .width(WIDTH)
    .style(theme::dialog);
    let layer =
        mouse_area(center(opaque(dialog)).style(theme::scrim)).on_press(Message::CancelRemove);
    stack([window, opaque(layer)]).into()
}

#[cfg(test)]
mod tests {
    use super::title;

    #[test]
    fn title_names_the_book_in_quotes() {
        assert_eq!(
            title("The Left Hand of Darkness"),
            "Remove “The Left Hand of Darkness”?"
        );
    }
}
