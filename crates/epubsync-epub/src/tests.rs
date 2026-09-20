//! Tests of the `Epub` methods that sit in `lib.rs`.

use crate::fixtures as common;
use crate::{Cover, Epub};

/// Opens the EPUB built from `opf` inside a temp folder. The folder is
/// returned with it, because it holds the file.
fn book(opf: &str) -> (tempfile::TempDir, Epub) {
    let dir = tempfile::tempdir().unwrap();
    let path = common::write_epub(dir.path(), "book.epub", opf);
    let epub = Epub::open(&path).unwrap();
    (dir, epub)
}

#[test]
fn an_epub_2_cover_meta_gives_the_cover_bytes() {
    let (_dir, epub) = book(common::EPUB2_OPF);
    assert_eq!(
        epub.cover().unwrap(),
        Cover::Image(common::COVER_JPEG.into())
    );
}

#[test]
fn an_epub_3_cover_image_property_gives_the_cover_bytes() {
    let (_dir, epub) = book(common::STANDARD_EBOOKS_OPF);
    assert_eq!(
        epub.cover().unwrap(),
        Cover::Image(common::COVER_JPEG.into())
    );
}

#[test]
fn a_file_that_names_no_cover_gives_none() {
    let (_dir, epub) = book(common::BARE_OPF);
    assert_eq!(epub.cover().unwrap(), Cover::None);
}

#[test]
fn a_cover_entry_the_zip_does_not_hold_gives_unreadable() {
    let (_dir, epub) = book(common::MISSING_COVER_OPF);
    let Cover::Unreadable(why) = epub.cover().unwrap() else {
        panic!("the missing entry read");
    };
    assert!(why.contains("no-such-cover.jpg"), "{why}");
}
