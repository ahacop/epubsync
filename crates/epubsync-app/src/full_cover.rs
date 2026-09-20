//! The full-size cover over the window. A click on the thumbnail in the
//! sidebar opens it, and a click anywhere or Escape closes it.
//!
//! The image is the one the publisher stored, read from the book file,
//! not the thumbnail the library holds. It is drawn at its own pixel
//! size when the window has room, and scaled down when it does not.

use iced::widget::{container, image, mouse_area, opaque, stack};
use iced::{ContentFit, Element, Fill, mouse};

use crate::Message;
use crate::theme;

const INSET: f32 = 44.0;

pub fn over<'a>(window: Element<'a, Message>, handle: &'a image::Handle) -> Element<'a, Message> {
    let picture = container(
        image(handle)
            .width(Fill)
            .height(Fill)
            .content_fit(ContentFit::ScaleDown),
    )
    .width(Fill)
    .height(Fill)
    .padding(INSET)
    .style(theme::scrim);
    let layer = mouse_area(picture)
        .interaction(mouse::Interaction::Pointer)
        .on_press(Message::HideCover);
    stack([window, opaque(layer)]).into()
}
