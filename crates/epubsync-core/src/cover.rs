//! The cover of a book as the library holds it: a small JPEG copy, or
//! the reason there is none.
//!
//! The library stores a thumbnail the way it stores the word count, so a
//! display reads one row and opens no zip. `Library::import` makes the
//! copy, and `Library::fill_covers` makes it for a book imported before
//! this table existed.
//!
//! Core keeps its own enum although it re-exports the EPUB crate's
//! `Metadata` and `Stats`. Those two are one type because the file and
//! the row hold the same value. A cover is not one value: the file holds
//! the image the publisher stored, and the row holds the thumbnail core
//! made from it. `Undecodable` needs the `image` crate, and `Unknown`
//! names a missing database row, so neither variant can live in the EPUB
//! crate. The match in `from_file` is the one place the two enums meet.

use std::io::Cursor;

use anyhow::{Result, bail};
use epubsync_epub::Cover as FileCover;

/// The longest side of a thumbnail, in pixels.
const MAX_SIDE: u32 = 480;

/// The JPEG quality of a thumbnail, out of 100.
const QUALITY: u8 = 85;

/// The three values a `covers` row holds. `Cover::from_row` reads the
/// same three back.
pub struct Row<'a> {
    pub state: &'static str,
    pub image: Option<&'a [u8]>,
    pub detail: Option<&'a str>,
}

/// A book's cover, or why the library holds none.
#[derive(Debug, Clone, PartialEq)]
pub enum Cover {
    /// The thumbnail, as JPEG bytes.
    Image(Vec<u8>),
    /// The book file names no cover.
    None,
    /// The file names a cover it does not give. The text is what the
    /// EPUB crate said.
    Unreadable(String),
    /// The cover bytes are not an image core decodes. The text is the
    /// decoder's message, which names the format.
    Undecodable(String),
    /// Core has not read this book's file. The book has no row.
    Unknown,
}

impl Cover {
    /// The cover to store for a book whose file gave `cover`. Bytes that
    /// decode become a thumbnail; bytes that do not become `Undecodable`
    /// with the decoder's message.
    pub fn from_file(cover: FileCover) -> Cover {
        match cover {
            FileCover::Image(bytes) => match thumbnail(&bytes) {
                Ok(small) => Cover::Image(small),
                Err(e) => Cover::Undecodable(format!("{e:#}")),
            },
            FileCover::None => Cover::None,
            FileCover::Unreadable(why) => Cover::Unreadable(why),
        }
    }

    /// The values of the `covers` row. `Unknown` is the one variant no
    /// row holds, so it has none to give.
    pub fn row(&self) -> Option<Row<'_>> {
        let row = |state, image, detail| {
            Some(Row {
                state,
                image,
                detail,
            })
        };
        match self {
            Cover::Image(bytes) => row("image", Some(bytes), None),
            Cover::None => row("none", None, None),
            Cover::Unreadable(why) => row("unreadable", None, Some(why)),
            Cover::Undecodable(why) => row("undecodable", None, Some(why)),
            Cover::Unknown => None,
        }
    }

    /// The variant a `covers` row holds. A state word the table does not
    /// allow is an error. The `CHECK` lines of the table make that
    /// unreachable, and the test for it stands guard over a later
    /// migration.
    pub fn from_row(state: &str, image: Option<Vec<u8>>, detail: Option<String>) -> Result<Cover> {
        Ok(match (state, image, detail) {
            ("image", Some(bytes), None) => Cover::Image(bytes),
            ("none", None, None) => Cover::None,
            ("unreadable", None, Some(why)) => Cover::Unreadable(why),
            ("undecodable", None, Some(why)) => Cover::Undecodable(why),
            (state, _, _) => bail!("a covers row reads {state}"),
        })
    }
}

/// A JPEG copy of `bytes` that fits inside a square of `MAX_SIDE`, with
/// the aspect ratio kept. An image already that small is encoded again
/// at the same size, so every stored cover is a JPEG whatever the
/// publisher used. The copy is RGB, because JPEG carries no alpha
/// channel, and an image with one loses it here.
pub fn thumbnail(bytes: &[u8]) -> Result<Vec<u8>> {
    let image = image::load_from_memory(bytes)?
        .thumbnail(MAX_SIDE, MAX_SIDE)
        .to_rgb8();
    let mut out = Cursor::new(Vec::new());
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, QUALITY).encode_image(&image)?;
    Ok(out.into_inner())
}
