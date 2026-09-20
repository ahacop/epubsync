//! The viewer's colors, fonts, and widget styles.
//!
//! The window follows the system light or dark mode. The app does not pick
//! a theme, so Iced picks its Light or Dark theme from the system, and every
//! style function here reads `theme.extended_palette().is_dark` to choose
//! between the two color sets. The `view` functions never see the mode: they
//! pass a closure that takes the theme, and Iced calls it when it draws.

use epubsync_core::device::ReadStatus;
use iced::font::Weight;
use iced::widget::{button, container, progress_bar, text, text_editor, text_input};
use iced::{Background, Border, Color, Element, Fill, Font, Shadow, Theme, Vector, border};

/// The interface typeface.
pub const SANS: Font = Font::with_name("Instrument Sans");
/// The typeface for the book title and the description.
pub const SERIF: Font = Font::with_name("Newsreader");
/// The typeface for ids, serials, and paths.
pub const MONO: Font = Font::with_name("JetBrains Mono");

pub const SANS_MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..SANS
};
pub const SANS_SEMIBOLD: Font = Font {
    weight: Weight::Semibold,
    ..SANS
};
pub const SERIF_MEDIUM: Font = Font {
    weight: Weight::Medium,
    ..SERIF
};

/// The text size of table cells, bylines, and labels.
pub const BODY: f32 = 13.5;

/// The height of the table header row and the sidebar header, which sit
/// side by side.
pub const HEADER: f32 = 32.0;

/// A small label in the medium weight: a column header, the sidebar's
/// "Book 13". It sets no color of its own, so a column header takes the
/// header button's color.
pub fn label<'a>(content: impl text::IntoFragment<'a>) -> text::Text<'a> {
    text(content).size(12).font(SANS_MEDIUM)
}

/// One color set. The names match the mockup's tokens.
#[derive(Debug, Clone, Copy)]
pub struct Colors {
    /// The window ground, the toolbar, the header row, the sidebar.
    pub window: Color,
    /// The table body, the filter field.
    pub surface: Color,
    /// A row under the pointer.
    pub surface_2: Color,
    /// Row and pane separators.
    pub line: Color,
    /// The filter field border.
    pub line_strong: Color,
    /// Text.
    pub ink: Color,
    /// The byline, the percent.
    pub ink_2: Color,
    /// Headers, secondary cells, labels.
    pub muted: Color,
    /// Placeholders, dashes, series numbers.
    pub faint: Color,
    /// The sort arrow, the selected row mark, the focus ring.
    pub accent: Color,
    /// The selected row ground.
    pub accent_tint: Color,
    pub reading: Color,
    pub reading_tint: Color,
    pub finished: Color,
    pub finished_tint: Color,
    pub unread: Color,
    pub unread_tint: Color,
    /// The ground of a button that deletes something, with white text.
    pub danger: Color,
    /// The same button under the pointer.
    pub danger_2: Color,
}

const fn hex(rgb: u32) -> Color {
    Color::from_rgb8((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8)
}

pub const LIGHT: Colors = Colors {
    window: hex(0xF6F7F5),
    surface: hex(0xFFFFFF),
    surface_2: hex(0xF1F3F0),
    line: hex(0xD9DDD9),
    line_strong: hex(0xC3C9C4),
    ink: hex(0x1C1F1E),
    ink_2: hex(0x4A524F),
    muted: hex(0x697270),
    faint: hex(0x98A09D),
    accent: hex(0x2B6777),
    accent_tint: hex(0xE3EEF1),
    reading: hex(0xB7791F),
    reading_tint: hex(0xF6ECD8),
    finished: hex(0x2E7D4F),
    finished_tint: hex(0xDFF0E4),
    unread: hex(0x8A918E),
    unread_tint: hex(0xECEEED),
    danger: hex(0xB42318),
    danger_2: hex(0x9A1D12),
};

pub const DARK: Colors = Colors {
    window: hex(0x1A1D1E),
    surface: hex(0x1F2324),
    surface_2: hex(0x262B2C),
    line: hex(0x2E3436),
    line_strong: hex(0x3C4447),
    ink: hex(0xE6E5E0),
    ink_2: hex(0xB8BCB8),
    muted: hex(0x8F9793),
    faint: hex(0x667070),
    accent: hex(0x7FB8C6),
    accent_tint: hex(0x1F3236),
    reading: hex(0xE0A94A),
    reading_tint: hex(0x3A2F16),
    finished: hex(0x5FBF85),
    finished_tint: hex(0x1B3426),
    unread: hex(0x7D8683),
    unread_tint: hex(0x2A2F30),
    danger: hex(0xD64545),
    danger_2: hex(0xE25C5C),
};

/// The color set for the theme Iced picked from the system.
pub fn colors(theme: &Theme) -> &'static Colors {
    if theme.extended_palette().is_dark {
        &DARK
    } else {
        &LIGHT
    }
}

impl Colors {
    /// The text color and the ground for a progress status.
    pub fn status(&self, status: ReadStatus) -> (Color, Color) {
        match status {
            ReadStatus::Reading => (self.reading, self.reading_tint),
            ReadStatus::Finished => (self.finished, self.finished_tint),
            ReadStatus::Unread => (self.unread, self.unread_tint),
        }
    }
}

/// The window ground and the default text color.
pub fn window(theme: &Theme) -> iced::theme::Style {
    let c = colors(theme);
    iced::theme::Style {
        background_color: c.window,
        text_color: c.ink,
    }
}

/// A text style that picks one color from the set, for
/// `text(...).style(theme::text_color(|c| c.muted))`.
pub fn text_color(pick: fn(&Colors) -> Color) -> impl Fn(&Theme) -> text::Style {
    move |theme| text::Style {
        color: Some(pick(colors(theme))),
    }
}

/// A container ground that picks one color from the set.
pub fn ground(pick: fn(&Colors) -> Color) -> impl Fn(&Theme) -> container::Style {
    move |theme| container::Style {
        background: Some(Background::Color(pick(colors(theme)))),
        ..container::Style::default()
    }
}

/// A 1 px `line` across the full width.
pub fn hline<'a, M: 'a>() -> Element<'a, M> {
    container(iced::widget::space())
        .width(Fill)
        .height(1)
        .style(ground(|c| c.line))
        .into()
}

/// A 1 px `line` down the full height.
pub fn vline<'a, M: 'a>() -> Element<'a, M> {
    container(iced::widget::space())
        .width(1)
        .height(Fill)
        .style(ground(|c| c.line))
        .into()
}

/// The status chip: the status word on its tint, with a 3 px radius.
pub fn chip(status: ReadStatus) -> impl Fn(&Theme) -> container::Style {
    move |theme| {
        let (color, tint) = colors(theme).status(status);
        container::Style {
            text_color: Some(color),
            background: Some(Background::Color(tint)),
            border: border::rounded(3),
            ..container::Style::default()
        }
    }
}

/// The progress bar: `reading` or `finished` on an `unread_tint` track.
pub fn bar(status: ReadStatus) -> impl Fn(&Theme) -> progress_bar::Style {
    move |theme| {
        let c = colors(theme);
        let fill = match status {
            ReadStatus::Finished => c.finished,
            ReadStatus::Reading | ReadStatus::Unread => c.reading,
        };
        progress_bar::Style {
            background: Background::Color(c.unread_tint),
            bar: Background::Color(fill),
            border: border::rounded(2),
        }
    }
}

/// The import strip's progress bar: `accent` on a `line` track.
pub fn import_bar(theme: &Theme) -> progress_bar::Style {
    let c = colors(theme);
    progress_bar::Style {
        background: Background::Color(c.line),
        bar: Background::Color(c.accent),
        border: border::rounded(2),
    }
}

/// A column header: no ground of its own. The label reads in `ink` on
/// the sorted column and under the pointer, and in `muted` elsewhere.
pub fn header(sorted: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = colors(theme);
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: None,
            text_color: if sorted || hovered { c.ink } else { c.muted },
            border: Border::default(),
            ..button::Style::default()
        }
    }
}

/// A toolbar tab: no ground of its own. The name reads in `ink` while
/// its pane is in view and under the pointer, and in `muted` elsewhere,
/// the same as a column header.
pub fn tab(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    header(active)
}

/// A table row. The selected row sits on `accent_tint`; a row under the
/// pointer sits on `surface_2`; the rest sit on `surface`.
pub fn row(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let c = colors(theme);
        let ground = if selected {
            c.accent_tint
        } else if matches!(status, button::Status::Hovered | button::Status::Pressed) {
            c.surface_2
        } else {
            c.surface
        };
        button::Style {
            background: Some(Background::Color(ground)),
            text_color: c.ink,
            border: Border::default(),
            ..button::Style::default()
        }
    }
}

/// The sidebar's close button: a bare glyph that gets a `surface_2` ground
/// under the pointer.
pub fn close(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors(theme);
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hovered.then_some(Background::Color(c.surface_2)),
        text_color: if hovered { c.ink } else { c.muted },
        border: border::rounded(5),
        ..button::Style::default()
    }
}

/// A toolbar button such as Reload: a `surface` box with a `line_strong`
/// border, the same shape as the filter field, that sits on `surface_2`
/// under the pointer. The label reads in `faint` while the button is off.
pub fn action(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors(theme);
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(Background::Color(if hovered {
            c.surface_2
        } else {
            c.surface
        })),
        text_color: if matches!(status, button::Status::Disabled) {
            c.faint
        } else {
            c.ink
        },
        border: Border {
            color: c.line_strong,
            width: 1.0,
            radius: 6.0.into(),
        },
        ..button::Style::default()
    }
}

/// The filter field: a `surface` box with a `line_strong` border that turns
/// `accent` while the field has focus.
pub fn filter(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let c = colors(theme);
    let focused = matches!(status, text_input::Status::Focused { .. });
    text_input::Style {
        background: Background::Color(c.surface),
        border: Border {
            color: if focused { c.accent } else { c.line_strong },
            width: 1.0,
            radius: 6.0.into(),
        },
        icon: c.muted,
        placeholder: c.faint,
        value: c.ink,
        selection: c.accent_tint,
    }
}

/// The description editor in the edit form: the filter field's colors
/// in a box of the same shape.
pub fn editor(theme: &Theme, status: text_editor::Status) -> text_editor::Style {
    let c = colors(theme);
    let focused = matches!(status, text_editor::Status::Focused { .. });
    text_editor::Style {
        background: Background::Color(c.surface),
        border: Border {
            color: if focused { c.accent } else { c.line_strong },
            width: 1.0,
            radius: 6.0.into(),
        },
        placeholder: c.faint,
        value: c.ink,
        selection: c.accent_tint,
    }
}

/// The button that commits a form, such as Save in the edit form: white
/// text on `accent`. The label reads in `faint` on `surface_2` while
/// the button is off.
pub fn primary(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors(theme);
    let (ground, ink) = match status {
        button::Status::Disabled => (c.surface_2, c.faint),
        _ => (c.accent, Color::WHITE),
    };
    button::Style {
        background: Some(Background::Color(ground)),
        text_color: ink,
        border: border::rounded(6),
        ..button::Style::default()
    }
}

/// A button that deletes something, such as Remove… in the sidebar and
/// Remove in the remove dialog: white text on `danger`, and on `danger_2`
/// under the pointer. The label reads in `faint` on `surface_2` while
/// the button is off.
pub fn danger(theme: &Theme, status: button::Status) -> button::Style {
    let c = colors(theme);
    let (ground, ink) = match status {
        button::Status::Disabled => (c.surface_2, c.faint),
        button::Status::Hovered | button::Status::Pressed => (c.danger_2, Color::WHITE),
        button::Status::Active => (c.danger, Color::WHITE),
    };
    button::Style {
        background: Some(Background::Color(ground)),
        text_color: ink,
        border: border::rounded(6),
        ..button::Style::default()
    }
}

/// The layer between the window and a dialog: black at half strength,
/// so the window shows through dimmed.
pub fn scrim(theme: &Theme) -> container::Style {
    let a = if theme.extended_palette().is_dark {
        0.6
    } else {
        0.45
    };
    container::Style {
        background: Some(Background::Color(Color { a, ..Color::BLACK })),
        ..container::Style::default()
    }
}

/// A dialog box: a `surface` panel with a `line_strong` border, rounded
/// corners, and a soft shadow.
pub fn dialog(theme: &Theme) -> container::Style {
    let c = colors(theme);
    container::Style {
        background: Some(Background::Color(c.surface)),
        text_color: Some(c.ink),
        border: Border {
            color: c.line_strong,
            width: 1.0,
            radius: 10.0.into(),
        },
        shadow: Shadow {
            color: Color {
                a: 0.3,
                ..Color::BLACK
            },
            offset: Vector::new(0.0, 8.0),
            blur_radius: 28.0,
        },
        ..container::Style::default()
    }
}
