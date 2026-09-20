//! Builds EPUB fixtures in memory for tests. Each test passes its own OPF
//! text, so one helper covers EPUB 2, EPUB 3, and every metadata form.

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

pub const OPF_PATH: &str = "OEBPS/content.opf";

pub const CONTAINER_XML: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"##;

pub const CHAPTER_XHTML: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title></head>
<body>
<h1>Chapter 1</h1>
<p>It was a dark and stormy night. The rain fell in torrents.</p>
</body>
</html>
"##;

/// A small EPUB 2 OPF in the shape Calibre writes.
pub const EPUB2_OPF: &str = r##"<?xml version='1.0' encoding='utf-8'?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="uuid_id" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="uuid_id" opf:scheme="uuid">a1b2c3</dc:identifier>
    <dc:title>The Left Hand of Darkness</dc:title>
    <dc:creator opf:file-as="Le Guin, Ursula K." opf:role="aut">Ursula K. Le Guin</dc:creator>
    <dc:language>en</dc:language>
    <dc:publisher>Ace Books</dc:publisher>
    <dc:description>&lt;p&gt;A novel of Winter.&lt;/p&gt;</dc:description>
    <meta name="calibre:series" content="Hainish Cycle"/>
    <meta name="calibre:series_index" content="4"/>
    <meta name="cover" content="cover"/>
  </metadata>
  <manifest>
    <item id="cover" href="cover.jpg" media-type="image/jpeg"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

pub const TOC_NCX: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/" version="2005-1">
  <head><meta name="dtb:uid" content="a1b2c3"/></head>
  <docTitle><text>Book</text></docTitle>
  <navMap>
    <navPoint id="n1" playOrder="1"><navLabel><text>Chapter 1</text></navLabel><content src="chapter1.xhtml"/></navPoint>
  </navMap>
</ncx>
"##;

/// A 1x1 JPEG the `image` crate decodes, enough for a cover entry.
pub const COVER_JPEG: &[u8] = &[
    0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0x4A, 0x46, 0x49, 0x46, 0x00, 0x01, 0x02, 0x00, 0x00, 0x01,
    0x00, 0x01, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11,
    0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xDB, 0x00, 0x43, 0x00, 0x05, 0x03, 0x04, 0x04,
    0x04, 0x03, 0x05, 0x04, 0x04, 0x04, 0x05, 0x05, 0x05, 0x06, 0x07, 0x0C, 0x08, 0x07, 0x07, 0x07,
    0x07, 0x0F, 0x0B, 0x0B, 0x09, 0x0C, 0x11, 0x0F, 0x12, 0x12, 0x11, 0x0F, 0x11, 0x11, 0x13, 0x16,
    0x1C, 0x17, 0x13, 0x14, 0x1A, 0x15, 0x11, 0x11, 0x18, 0x21, 0x18, 0x1A, 0x1D, 0x1D, 0x1F, 0x1F,
    0x1F, 0x13, 0x17, 0x22, 0x24, 0x22, 0x1E, 0x24, 0x1C, 0x1E, 0x1F, 0x1E, 0xFF, 0xDB, 0x00, 0x43,
    0x01, 0x05, 0x05, 0x05, 0x07, 0x06, 0x07, 0x0E, 0x08, 0x08, 0x0E, 0x1E, 0x14, 0x11, 0x14, 0x1E,
    0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E,
    0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E,
    0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E, 0x1E,
    0x1E, 0xFF, 0xC4, 0x00, 0x1F, 0x00, 0x00, 0x01, 0x05, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
    0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x10, 0x00, 0x02, 0x01, 0x03, 0x03, 0x02, 0x04, 0x03, 0x05,
    0x05, 0x04, 0x04, 0x00, 0x00, 0x01, 0x7D, 0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21,
    0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07, 0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xA1, 0x08, 0x23,
    0x42, 0xB1, 0xC1, 0x15, 0x52, 0xD1, 0xF0, 0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0A, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A,
    0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A,
    0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A,
    0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99,
    0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6, 0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7,
    0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4, 0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5,
    0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE1, 0xE2, 0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF1,
    0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFF, 0xC4, 0x00, 0x1F, 0x01, 0x00, 0x03,
    0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
    0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0xFF, 0xC4, 0x00, 0xB5, 0x11, 0x00,
    0x02, 0x01, 0x02, 0x04, 0x04, 0x03, 0x04, 0x07, 0x05, 0x04, 0x04, 0x00, 0x01, 0x02, 0x77, 0x00,
    0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71, 0x13,
    0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xA1, 0xB1, 0xC1, 0x09, 0x23, 0x33, 0x52, 0xF0, 0x15,
    0x62, 0x72, 0xD1, 0x0A, 0x16, 0x24, 0x34, 0xE1, 0x25, 0xF1, 0x17, 0x18, 0x19, 0x1A, 0x26, 0x27,
    0x28, 0x29, 0x2A, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4A, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6A, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88,
    0x89, 0x8A, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9A, 0xA2, 0xA3, 0xA4, 0xA5, 0xA6,
    0xA7, 0xA8, 0xA9, 0xAA, 0xB2, 0xB3, 0xB4, 0xB5, 0xB6, 0xB7, 0xB8, 0xB9, 0xBA, 0xC2, 0xC3, 0xC4,
    0xC5, 0xC6, 0xC7, 0xC8, 0xC9, 0xCA, 0xD2, 0xD3, 0xD4, 0xD5, 0xD6, 0xD7, 0xD8, 0xD9, 0xDA, 0xE2,
    0xE3, 0xE4, 0xE5, 0xE6, 0xE7, 0xE8, 0xE9, 0xEA, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9,
    0xFA, 0xFF, 0xDA, 0x00, 0x0C, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3F, 0x00, 0xF3,
    0x4A, 0xFB, 0x23, 0xE7, 0x4F, 0xFF, 0xD9,
];

/// Builds an EPUB zip in memory with `mimetype` first and stored, then
/// `container.xml`, the OPF at `OEBPS/content.opf`, the chapter
/// `CHAPTER_XHTML`, an NCX, and a cover image.
pub fn build_epub(opf: &str) -> Vec<u8> {
    build_book(opf, CHAPTER_XHTML)
}

/// Builds an EPUB zip like `build_epub`, with `chapter` as the one XHTML
/// chapter, so a test can build a book with a known word count.
pub fn build_book(opf: &str, chapter: &str) -> Vec<u8> {
    build_book_with_cover(opf, chapter, COVER_JPEG)
}

/// Builds an EPUB zip like `build_book`, with `cover` as the bytes of the
/// cover entry, so a test can build a book whose cover is not an image.
pub fn build_book_with_cover(opf: &str, chapter: &str, cover: &[u8]) -> Vec<u8> {
    let mut zw = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zw.start_file("mimetype", stored).unwrap();
    zw.write_all(b"application/epub+zip").unwrap();
    zw.start_file("META-INF/container.xml", deflated).unwrap();
    zw.write_all(CONTAINER_XML.as_bytes()).unwrap();
    zw.start_file(OPF_PATH, deflated).unwrap();
    zw.write_all(opf.as_bytes()).unwrap();
    zw.start_file("OEBPS/chapter1.xhtml", deflated).unwrap();
    zw.write_all(chapter.as_bytes()).unwrap();
    zw.start_file("OEBPS/toc.ncx", deflated).unwrap();
    zw.write_all(TOC_NCX.as_bytes()).unwrap();
    zw.start_file("OEBPS/cover.jpg", stored).unwrap();
    zw.write_all(cover).unwrap();
    zw.finish().unwrap().into_inner()
}

/// Writes the EPUB built from `opf` to `name` inside `dir` and returns its path.
pub fn write_epub(dir: &Path, name: &str, opf: &str) -> PathBuf {
    write_book(dir, name, opf, CHAPTER_XHTML)
}

/// Writes the EPUB built from `opf` and `chapter` to `name` inside `dir`.
pub fn write_book(dir: &Path, name: &str, opf: &str, chapter: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, build_book(opf, chapter)).unwrap();
    path
}

/// Writes the EPUB built from `opf` to `name` inside `dir`, with `cover`
/// as the bytes of the cover entry.
pub fn write_book_with_cover(dir: &Path, name: &str, opf: &str, cover: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, build_book_with_cover(opf, CHAPTER_XHTML, cover)).unwrap();
    path
}

/// Replaces the OPF entry of the EPUB at `path` with `text`, as written.
pub fn replace_opf(path: &Path, text: &str) -> anyhow::Result<()> {
    crate::archive::rewrite(path, OPF_PATH, text)
}

/// Reads one entry out of a zip file as a string.
pub fn read_entry(path: &Path, name: &str) -> String {
    let file = std::fs::File::open(path).unwrap();
    let mut archive = zip::ZipArchive::new(file).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let mut text = String::new();
    std::io::Read::read_to_string(&mut entry, &mut text).unwrap();
    text
}

/// An EPUB 3 OPF with refinements and a `belongs-to-collection` series.
pub const EPUB3_OPF: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id" xml:lang="en">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">urn:uuid:d4e5f6</dc:identifier>
    <dc:title id="t1">A Wizard of Earthsea</dc:title>
    <dc:creator id="creator01">Ursula K. Le Guin</dc:creator>
    <meta refines="#creator01" property="role" scheme="marc:relators">aut</meta>
    <meta refines="#creator01" property="file-as">Le Guin, Ursula K.</meta>
    <dc:language>en</dc:language>
    <dc:publisher>Parnassus Press</dc:publisher>
    <dc:description>Ged the sparrowhawk.</dc:description>
    <meta property="dcterms:modified">2024-01-01T00:00:00Z</meta>
    <meta property="belongs-to-collection" id="c01">Earthsea Cycle</meta>
    <meta refines="#c01" property="collection-type">series</meta>
    <meta refines="#c01" property="group-position">1</meta>
  </metadata>
  <manifest>
    <item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

/// An EPUB 3 OPF as Calibre writes it: both `file-as` forms on one creator
/// and the Calibre series metas.
pub const EPUB3_CALIBRE_OPF: &str = r##"<?xml version='1.0' encoding='utf-8'?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="uuid_id" version="3.0" prefix="calibre: https://calibre-ebook.com">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="uuid_id">urn:uuid:112233</dc:identifier>
    <dc:title>The Dispossessed</dc:title>
    <dc:creator id="id" opf:file-as="Le Guin, Ursula K." opf:role="aut">Ursula K. Le Guin</dc:creator>
    <meta refines="#id" property="file-as">Le Guin, Ursula K.</meta>
    <meta refines="#id" property="role" scheme="marc:relators">aut</meta>
    <dc:language>en</dc:language>
    <dc:publisher>Harper &amp; Row</dc:publisher>
    <dc:description>An ambiguous utopia.</dc:description>
    <meta name="calibre:series" content="Hainish Cycle"/>
    <meta name="calibre:series_index" content="5"/>
    <meta property="dcterms:modified">2024-01-01T00:00:00Z</meta>
  </metadata>
  <manifest>
    <item id="cover" href="cover.jpg" media-type="image/jpeg" properties="cover-image"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

/// An EPUB 2 OPF whose `description` carries the Dublin Core namespace as
/// its own default namespace instead of the `dc:` prefix.
pub const DEFAULT_NS_DESCRIPTION_OPF: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="bookid" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="bookid">isbn-0</dc:identifier>
    <dc:title>The Lathe of Heaven</dc:title>
    <dc:creator opf:file-as="Le Guin, Ursula K.">Ursula K. Le Guin</dc:creator>
    <dc:language>en</dc:language>
    <description xmlns="http://purl.org/dc/elements/1.1/">Dreams that change the world.</description>
  </metadata>
  <manifest>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

/// An EPUB 3 OPF with two titles, the second marked `main`.
pub const TWO_TITLES_OPF: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="pub-id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="pub-id">urn:uuid:aa</dc:identifier>
    <dc:title id="t1">Earthsea</dc:title>
    <meta refines="#t1" property="title-type">collection</meta>
    <dc:title id="t2">The Tombs of Atuan</dc:title>
    <meta refines="#t2" property="title-type">main</meta>
    <dc:creator id="a1">Ursula K. Le Guin</dc:creator>
    <dc:language>en</dc:language>
  </metadata>
  <manifest>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

/// An EPUB 2 OPF with no series, no publisher, no description, no `file-as`,
/// and no `opf` prefix declared.
pub const BARE_OPF: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="bookid" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">isbn-1</dc:identifier>
    <dc:title>Candide</dc:title>
    <dc:creator>Voltaire</dc:creator>
    <dc:language>fr</dc:language>
  </metadata>
  <manifest>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

/// An EPUB 2 OPF whose cover item names an entry the zip does not hold.
pub const MISSING_COVER_OPF: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="bookid" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="bookid">isbn-2</dc:identifier>
    <dc:title>The Word for World Is Forest</dc:title>
    <dc:creator opf:file-as="Le Guin, Ursula K.">Ursula K. Le Guin</dc:creator>
    <dc:language>en</dc:language>
    <meta name="cover" content="cover"/>
  </metadata>
  <manifest>
    <item id="cover" href="no-such-cover.jpg" media-type="image/jpeg"/>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

/// An EPUB 3 OPF in the shape Standard Ebooks writes: tab indentation,
/// refinements on the publisher, and the word count and reading ease.
pub const STANDARD_EBOOKS_OPF: &str = r##"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" dir="ltr" prefix="rdf: http://www.w3.org/1999/02/22-rdf-syntax-ns#" unique-identifier="uid" version="3.0" xml:lang="en-US">
	<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
		<dc:identifier id="uid">https://standardebooks.org/ebooks/jane-austen/pride-and-prejudice</dc:identifier>
		<dc:date>2014-05-25T00:00:00Z</dc:date>
		<meta property="dcterms:modified">2014-05-25T00:00:00Z</meta>
		<meta property="rdf:type">http://schema.org/Book</meta>
		<dc:publisher id="publisher">Standard Ebooks</dc:publisher>
		<meta property="file-as" refines="#publisher">Standard Ebooks</meta>
		<meta property="role" refines="#publisher" scheme="marc:relators">bkd</meta>
		<dc:description>
			&lt;p&gt;A novel of manners.&lt;/p&gt;
		</dc:description>
		<dc:language>en-GB</dc:language>
		<meta property="schema:wordCount">121970</meta>
		<meta property="schema:educationalLevel">60.95</meta>
		<dc:title id="title">Pride and Prejudice</dc:title>
		<meta property="file-as" refines="#title">Pride and Prejudice</meta>
		<dc:creator id="author">Jane Austen</dc:creator>
		<meta property="file-as" refines="#author">Austen, Jane</meta>
		<meta property="role" refines="#author" scheme="marc:relators">aut</meta>
	</metadata>
	<manifest>
		<item href="cover.jpg" id="cover" media-type="image/jpeg" properties="cover-image"/>
		<item href="chapter1.xhtml" id="ch1" media-type="application/xhtml+xml"/>
	</manifest>
	<spine>
		<itemref idref="ch1"/>
	</spine>
</package>
"##;

pub const ALL_OPFS: &[(&str, &str)] = &[
    ("epub2", EPUB2_OPF),
    ("epub3", EPUB3_OPF),
    ("epub3-calibre", EPUB3_CALIBRE_OPF),
    ("default-ns-description", DEFAULT_NS_DESCRIPTION_OPF),
    ("two-titles", TWO_TITLES_OPF),
    ("bare", BARE_OPF),
    ("standard-ebooks", STANDARD_EBOOKS_OPF),
];

/// Flips a byte inside the compressed data of the named entry, so reading
/// that entry fails its CRC or its inflate.
pub fn corrupt_entry(path: &Path, name: &str) {
    let mut bytes = std::fs::read(path).unwrap();
    let name_at = bytes
        .windows(name.len())
        .position(|w| w == name.as_bytes())
        .unwrap();
    let header = name_at - 30;
    assert_eq!(&bytes[header..header + 4], b"PK\x03\x04");
    let extra_len = u16::from_le_bytes([bytes[header + 28], bytes[header + 29]]) as usize;
    let data = name_at + name.len() + extra_len;
    bytes[data + 5] ^= 0xFF;
    std::fs::write(path, bytes).unwrap();
}
