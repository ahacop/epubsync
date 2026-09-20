use std::path::Path;

use epubsync_core::config::{self, Config};
use epubsync_core::cover::{self, Cover};
use epubsync_core::library::{Book, Field, ImportOutcome, Library, Stats};
use epubsync_core::metadata::{Author, Metadata, Series};
use epubsync_epub::Epub;
use epubsync_epub::fixtures as common;

struct Setup {
    _dir: tempfile::TempDir,
    root: std::path::PathBuf,
    config: Config,
}

/// Makes a temp folder and inits a library in a `library` subfolder.
fn setup() -> Setup {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let lib = Library::init(&root.join("library")).unwrap();
    let config = Config {
        library: lib.folder.clone(),
    };
    drop(lib);
    Setup {
        _dir: dir,
        root,
        config,
    }
}

/// The names of the `.tmp` files in a folder. An import that has ended
/// leaves none.
fn temp_files(folder: &Path) -> Vec<String> {
    std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect()
}

/// The first author's sort name as the file gives it, `None` when the
/// file gives none.
fn file_sort(path: &Path) -> Option<String> {
    let (record, made) = Epub::open(path).unwrap().metadata(str::to_string);
    made.is_empty().then(|| record.authors[0].sort.clone())
}

#[test]
fn init_then_open_and_a_second_open_fails_on_the_lock() {
    let s = setup();
    assert!(s.config.library.join("library.sqlite").exists());
    let config_path = s.root.join("config.toml");
    config::save(&config_path, &s.config).unwrap();
    assert_eq!(
        toml::from_str::<Config>(&std::fs::read_to_string(&config_path).unwrap())
            .unwrap()
            .library,
        s.config.library
    );

    let first = Library::open(&s.config).unwrap();
    let err = match Library::open(&s.config) {
        Ok(_) => panic!("second open took the lock"),
        Err(e) => e,
    };
    assert!(
        err.to_string().contains("another EpubSync is running"),
        "{err}"
    );
    drop(first);
    Library::open(&s.config).unwrap();
}

#[test]
fn imports_an_epub_and_writes_the_made_sort_name() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "candide.epub", common::BARE_OPF);

    let outcome = lib.import(&source, false).unwrap();
    let ImportOutcome::Imported { id, made_sort } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(id, 1);
    assert_eq!(
        made_sort,
        vec![Author {
            name: "Voltaire".into(),
            sort: "Voltaire".into()
        }]
    );

    let path = lib.book_path(id);
    assert_eq!(path.file_name().unwrap(), "1.kepub.epub");
    assert!(path.exists());
    assert!(temp_files(&lib.folder).is_empty());
    assert!(common::read_entry(&path, "OEBPS/chapter1.xhtml").contains("koboSpan"));
    assert_eq!(file_sort(&path).as_deref(), Some("Voltaire"));

    let books = lib.list().unwrap();
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].revision, 1);
    assert_eq!(books[0].metadata.title, "Candide");
    assert_eq!(books[0].metadata.authors[0].sort, "Voltaire");
    assert!(books[0].metadata.series.is_none());

    // The source file is unchanged.
    assert_eq!(
        std::fs::read(&source).unwrap(),
        common::build_epub(common::BARE_OPF)
    );
}

#[test]
fn import_reads_every_field_and_leaves_a_file_with_sort_names_alone() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, made_sort } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    assert!(made_sort.is_empty());
    let book = lib.get(id).unwrap();
    assert_eq!(book.metadata.title, "The Left Hand of Darkness");
    assert_eq!(
        book.metadata.authors,
        vec![Author {
            name: "Ursula K. Le Guin".into(),
            sort: "Le Guin, Ursula K.".into()
        }]
    );
    assert_eq!(
        book.metadata.series,
        Some(Series {
            name: "Hainish Cycle".into(),
            number: Some(4.0)
        })
    );
    assert_eq!(book.metadata.publisher.as_deref(), Some("Ace Books"));
    assert_eq!(
        book.metadata.description.as_deref(),
        Some("<p>A novel of Winter.</p>")
    );
}

/// A chapter with eleven words in two sentences.
const SHORT_CHAPTER: &str = r##"<?xml version="1.0" encoding="UTF-8"?>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Chapter 1</title></head>
<body>
<p>The cat sat on the mat. It was a <i>sunny</i> day.</p>
</body>
</html>
"##;

const SHORT_STATS: Stats = Stats {
    word_count: Some(11),
    reading_ease: Some(109.0),
};

#[test]
fn import_keeps_the_numbers_a_standard_ebooks_file_carries() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "pp.epub", common::STANDARD_EBOOKS_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    assert_eq!(
        lib.get(id).unwrap().stats,
        Stats {
            word_count: Some(121970),
            reading_ease: Some(60.95),
        }
    );
    // The file keeps the numbers Standard Ebooks wrote.
    let text = common::read_entry(&lib.book_path(id), common::OPF_PATH);
    assert!(text.contains(r#"<meta property="schema:wordCount">121970</meta>"#));
    assert!(text.contains(r#"<meta property="schema:educationalLevel">60.95</meta>"#));
}

#[test]
fn import_measures_a_file_with_no_word_count() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_book(&s.root, "lhod.epub", common::EPUB2_OPF, SHORT_CHAPTER);
    let ImportOutcome::Imported { id, made_sort } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    assert!(made_sort.is_empty());
    assert_eq!(lib.get(id).unwrap().stats, SHORT_STATS);

    // A book in a language the crate has no Flesch coefficients for gets
    // a word count and no reading ease.
    let latin = common::EPUB2_OPF
        .replace(
            "<dc:language>en</dc:language>",
            "<dc:language>la</dc:language>",
        )
        .replace("The Left Hand of Darkness", "De Bello Gallico");
    let source = common::write_book(&s.root, "bello.epub", &latin, SHORT_CHAPTER);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    assert_eq!(
        lib.get(id).unwrap().stats,
        Stats {
            word_count: Some(11),
            reading_ease: None,
        }
    );
}

#[test]
fn import_writes_the_measured_numbers_into_the_library_file() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_book(&s.root, "lhod.epub", common::EPUB2_OPF, SHORT_CHAPTER);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    let path = lib.book_path(id);
    // The conversion re-indents the OPF, so the lines are matched trimmed.
    let text = common::read_entry(&path, common::OPF_PATH);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let at = lines
        .iter()
        .position(|l| *l == r#"<meta property="schema:wordCount">11</meta>"#)
        .unwrap_or_else(|| panic!("{text}"));
    assert_eq!(
        lines[at + 1],
        r#"<meta property="schema:educationalLevel">109.00</meta>"#,
        "{text}"
    );
    assert_eq!(Epub::open(&path).unwrap().stats(), SHORT_STATS);
    assert!(common::read_entry(&path, "OEBPS/chapter1.xhtml").contains("koboSpan"));

    // An edit writes the same numbers again.
    let mut record = lib.get(id).unwrap().metadata;
    record.title = "The Left Hand".into();
    lib.edit(id, &record).unwrap();
    assert_eq!(
        common::read_entry(&path, common::OPF_PATH),
        text.replace("The Left Hand of Darkness", "The Left Hand")
    );
}

#[test]
fn stops_on_the_same_title_and_author_unless_forced() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    assert_eq!(
        lib.import(&source, false).unwrap(),
        ImportOutcome::Exists { id }
    );
    assert_eq!(lib.list().unwrap().len(), 1);

    let ImportOutcome::Imported { id: second, .. } = lib.import(&source, true).unwrap() else {
        panic!();
    };
    assert_ne!(second, id);
    assert_eq!(lib.list().unwrap().len(), 2);
    assert!(lib.book_path(second).exists());
}

#[test]
fn copies_a_kepub_without_converting() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.kepub.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    // The chapter is copied as it is. The OPF gets the measured numbers.
    let unconverted = |path: &Path| {
        assert_eq!(
            common::read_entry(path, "OEBPS/chapter1.xhtml"),
            common::CHAPTER_XHTML
        );
        assert!(
            common::read_entry(path, common::OPF_PATH)
                .contains(r#"<meta property="schema:wordCount">13</meta>"#)
        );
    };
    unconverted(&lib.book_path(id));

    // A file named .kepub, as some publishers ship Kobo builds, is a KEPUB too.
    let other = common::EPUB2_OPF.replace("The Left Hand of Darkness", "The Dispossessed");
    let source = common::write_epub(&s.root, "other.kepub", &other);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    unconverted(&lib.book_path(id));
}

#[test]
fn a_failed_conversion_leaves_no_row_and_no_temp_file() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    // The chapter is in the manifest and not in the spine, so measuring
    // skips it and the conversion is the first step that reads it.
    let opf = common::EPUB2_OPF.replace(r#"<itemref idref="ch1"/>"#, "");
    let source = common::write_epub(&s.root, "broken.epub", &opf);
    common::corrupt_entry(&source, "OEBPS/chapter1.xhtml");
    let err = lib.import(&source, false).unwrap_err();
    assert!(err.to_string().contains("convert"), "{err}");
    assert!(lib.list().unwrap().is_empty());
    assert!(temp_files(&lib.folder).is_empty());
}

#[test]
fn a_failure_after_the_conversion_leaves_no_temp_file() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    // A folder at the book's path makes the rename of the converted
    // file fail, after the conversion and the row insert.
    std::fs::create_dir(lib.book_path(1)).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let err = lib.import(&source, false).unwrap_err();
    assert!(err.to_string().contains("rename"), "{err}");
    assert!(temp_files(&lib.folder).is_empty());
}

#[test]
fn edit_updates_the_row_the_revision_and_the_file() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    let mut record = lib.get(id).unwrap().metadata;
    record.title = "The Left Hand".into();
    record.authors[0].sort = "Le Guin, U. K.".into();
    record.series = Some(Series {
        name: "Hainish".into(),
        number: Some(4.5),
    });
    record.publisher = None;
    lib.edit(id, &record).unwrap();

    let book = lib.get(id).unwrap();
    assert_eq!(book.revision, 2);
    assert_eq!(book.metadata, record);

    let (file, made) = Epub::open(&lib.book_path(id))
        .unwrap()
        .metadata(str::to_string);
    assert!(made.is_empty());
    assert_eq!(file, record);
    assert!(common::read_entry(&lib.book_path(id), "OEBPS/chapter1.xhtml").contains("koboSpan"));
    assert_eq!(
        common::read_entry(&lib.book_path(id), "mimetype"),
        "application/epub+zip"
    );
}

#[test]
fn fields_lists_the_details_the_book_has() {
    let mut book = Book {
        id: 1,
        revision: 1,
        metadata: Metadata {
            title: "Can You Forgive Her?".into(),
            publisher: Some("Chapman & Hall".into()),
            series: Some(Series {
                name: "Palliser".into(),
                number: Some(1.0),
            }),
            ..Metadata::default()
        },
        stats: Stats {
            word_count: Some(121_970),
            reading_ease: Some(60.95),
        },
    };
    let series = book.metadata.series.clone().unwrap();
    assert_eq!(
        book.fields(),
        [
            Field::Publisher("Chapman & Hall"),
            Field::Series(&series),
            Field::WordCount(121_970),
            Field::ReadingEase(60.95),
        ]
    );

    book.metadata.series = None;
    book.stats.reading_ease = None;
    assert_eq!(
        book.fields(),
        [
            Field::Publisher("Chapman & Hall"),
            Field::WordCount(121_970)
        ]
    );
}

#[test]
fn book_progress_reads_one_book_in_device_order() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    lib.db
        .execute_batch(&format!(
            "INSERT INTO devices (serial) VALUES ('N1'), ('N2');
             INSERT INTO progress_history (book_id, device_serial, percent, status, last_read, time_spent, finished_at, seen_at) VALUES
               ({id}, 'N2', 100, 2, NULL, NULL, '2026-08-01', '2026-08-02'),
               ({id}, 'N1', 20, 1, '2026-08-20', 600, NULL, '2026-08-21'),
               ({id}, 'N1', 37, 1, '2026-09-01', 1200, NULL, '2026-09-02');"
        ))
        .unwrap();

    // The current progress is the newest row per device.
    let rows = lib.book_progress(id).unwrap();
    let serials: Vec<&str> = rows.iter().map(|p| p.device_serial.as_str()).collect();
    assert_eq!(serials, ["N1", "N2"]);
    assert_eq!(rows[0].percent, 37);
    assert_eq!(rows[0].last_read.as_deref(), Some("2026-09-01"));
    assert_eq!(rows[0].time_spent, Some(1200));
    assert_eq!(rows[0].finished_at, None);
    assert_eq!(rows[1].finished_at.as_deref(), Some("2026-08-01"));
    assert!(lib.book_progress(id + 1).unwrap().is_empty());
    let all = lib.progress().unwrap();
    assert_eq!(all[&id], rows);

    // The history is every row, oldest first.
    let history = lib.book_history(id).unwrap();
    let summary: Vec<(&str, i64, &str)> = history
        .iter()
        .map(|h| (h.device_serial.as_str(), h.percent, h.seen_day()))
        .collect();
    assert_eq!(
        summary,
        [
            ("N2", 100, "2026-08-02"),
            ("N1", 20, "2026-08-21"),
            ("N1", 37, "2026-09-02")
        ]
    );
    assert_eq!(history[1].time_spent, Some(600));
    assert!(lib.book_history(id + 1).unwrap().is_empty());
}

#[test]
fn remove_deletes_the_file_and_marks_the_book_row() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    lib.db
        .execute("INSERT INTO devices (serial) VALUES ('N123')", [])
        .unwrap();
    lib.db
        .execute(
            "INSERT INTO sent (book_id, device_serial, revision) VALUES (?1, 'N123', 1)",
            [id],
        )
        .unwrap();
    lib.db
        .execute(
            "INSERT INTO progress_history (book_id, device_serial, percent, status, last_read, seen_at)
             VALUES (?1, 'N123', 50, 1, '2026-01-01', '2026-01-02')",
            [id],
        )
        .unwrap();
    lib.db
        .execute(
            "INSERT INTO words (word, device_serial, book_id, dict_suffix, looked_up_at)
             VALUES ('ansible', 'N123', ?1, '-en', '2026-01-01')",
            [id],
        )
        .unwrap();

    let path = lib.book_path(id);
    lib.remove(id).unwrap();
    assert!(!path.exists());
    assert!(lib.list().unwrap().is_empty());
    assert!(lib.get(id).is_err());
    let count = |table: &str| -> i64 {
        lib.db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count("books"), 1);
    assert_eq!(count("book_authors"), 1);
    assert_eq!(count("book_stats"), 1);
    assert_eq!(count("sent"), 0);
    assert_eq!(count("progress_history"), 1);
    assert_eq!(lib.book_history(id).unwrap().len(), 1);
    let deleted_at: Option<String> = lib
        .db
        .query_row("SELECT deleted_at FROM books WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(deleted_at.is_some());
    let words = lib.words(Some(id), None).unwrap();
    assert_eq!(words.len(), 1);
    assert_eq!(words[0].book_title, "The Left Hand of Darkness");
    assert!(lib.remove(id).is_err());
}

#[test]
fn a_new_book_never_takes_the_id_of_a_removed_one() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id: first, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    lib.remove(first).unwrap();
    let ImportOutcome::Imported { id: second, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    assert!(second > first, "{second} after {first}");
}

#[test]
fn the_migration_starts_book_ids_above_the_ids_word_rows_hold() {
    let s = setup();
    // A database at migration 1: book 1 is in the library, and a word row
    // holds book 5, which was removed.
    std::fs::remove_file(s.config.library.join("library.sqlite")).unwrap();
    let db = rusqlite::Connection::open(s.config.library.join("library.sqlite")).unwrap();
    db.execute_batch(include_str!("../src/migrations/1-tables.sql"))
        .unwrap();
    db.execute_batch(
        "PRAGMA user_version = 1;
         INSERT INTO books (id, title) VALUES (1, 'Kept');
         INSERT INTO book_authors (book_id, position, name, sort) VALUES (1, 0, 'A', 'A');
         INSERT INTO words (word, device_serial, book_id, volume_id, looked_up_at)
             VALUES ('ansible', 'N123', 5, 'file:///mnt/onboard/EpubSync/5.kepub.epub', '2026-01-01');",
    )
    .unwrap();
    drop(db);

    let mut lib = Library::open(&s.config).unwrap();
    assert_eq!(lib.get(1).unwrap().metadata.title, "Kept");
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!();
    };
    assert_eq!(id, 6);
}

#[test]
fn the_migration_drops_word_rows_without_a_book() {
    let s = setup();
    // A database at migration 2: book 1 is in the library. One word row
    // holds book 1, one holds book 5, which was removed, and one holds no
    // book.
    std::fs::remove_file(s.config.library.join("library.sqlite")).unwrap();
    let db = rusqlite::Connection::open(s.config.library.join("library.sqlite")).unwrap();
    db.execute_batch(include_str!("../src/migrations/1-tables.sql"))
        .unwrap();
    db.execute_batch(include_str!("../src/migrations/2-books-autoincrement.sql"))
        .unwrap();
    db.execute_batch(
        "PRAGMA user_version = 2;
         INSERT INTO books (id, title) VALUES (1, 'Kept');
         INSERT INTO words (word, device_serial, book_id, volume_id, book_title, looked_up_at) VALUES
             ('ansible', 'N123', 1, 'file:///mnt/onboard/EpubSync/1.kepub.epub', 'Old title', '2026-01-01'),
             ('kemmer', 'N123', 5, 'file:///mnt/onboard/EpubSync/5.kepub.epub', 'Gone', '2026-01-02'),
             ('shifgrethor', 'N123', NULL, 'store-volume', 'A Store Book', '2026-01-03');",
    )
    .unwrap();
    drop(db);

    let lib = Library::open(&s.config).unwrap();
    let words = lib.words(None, None).unwrap();
    assert_eq!(words.len(), 1);
    assert_eq!(
        (
            words[0].word.as_str(),
            words[0].book_id,
            words[0].book_title.as_str()
        ),
        ("ansible", 1, "Kept")
    );
}

#[test]
fn the_migration_copies_progress_into_the_history() {
    let s = setup();
    // A database at migration 3: two books, one finished on N1 and one
    // being read on N1, and a progress row for a book with no row.
    std::fs::remove_file(s.config.library.join("library.sqlite")).unwrap();
    let db = rusqlite::Connection::open(s.config.library.join("library.sqlite")).unwrap();
    for sql in [
        include_str!("../src/migrations/1-tables.sql"),
        include_str!("../src/migrations/2-books-autoincrement.sql"),
        include_str!("../src/migrations/3-deleted-books.sql"),
    ] {
        db.execute_batch(sql).unwrap();
    }
    db.execute_batch(
        "PRAGMA user_version = 3;
         INSERT INTO books (id, title) VALUES (1, 'Done'), (2, 'Open');
         INSERT INTO devices (serial) VALUES ('N1');
         INSERT INTO progress (book_id, device_serial, percent, status, last_read) VALUES
           (1, 'N1', 100, 2, '2026-05-12T10:00:00Z'),
           (2, 'N1', 37, 1, '2026-09-01T10:00:00Z'),
           (9, 'N1', 5, 1, NULL);",
    )
    .unwrap();
    drop(db);

    let lib = Library::open(&s.config).unwrap();
    let progress = lib.progress().unwrap();
    assert_eq!(progress.len(), 2);
    let done = &progress[&1][0];
    assert_eq!(done.percent, 100);
    assert_eq!(done.finished_at.as_deref(), Some("2026-05-12T10:00:00Z"));
    assert_eq!(done.time_spent, None);
    let open = &progress[&2][0];
    assert_eq!(open.percent, 37);
    assert_eq!(open.finished_at, None);
    let history = lib.book_history(1).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].seen_at.len(), "2026-09-19T12:00:00Z".len());
    assert!(lib.book_history(9).unwrap().is_empty());
}

#[test]
fn an_import_stores_the_thumbnail_of_the_cover() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!("not imported");
    };

    let Cover::Image(bytes) = lib.cover(id).unwrap() else {
        panic!("{:?}", lib.cover(id).unwrap());
    };
    assert_eq!(&bytes[..3], b"\xFF\xD8\xFF");
    assert_eq!(bytes, cover::thumbnail(common::COVER_JPEG).unwrap());
}

#[test]
fn a_book_whose_file_names_no_cover_gives_none() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "candide.epub", common::BARE_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!("not imported");
    };
    assert_eq!(lib.cover(id).unwrap(), Cover::None);
}

#[test]
fn a_cover_the_file_does_not_hold_imports_and_gives_unreadable() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "forest.epub", common::MISSING_COVER_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!("not imported");
    };
    let Cover::Unreadable(why) = lib.cover(id).unwrap() else {
        panic!("{:?}", lib.cover(id).unwrap());
    };
    assert!(why.contains("no-such-cover.jpg"), "{why}");
}

#[test]
fn a_cover_entry_that_is_not_an_image_gives_undecodable() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_book_with_cover(
        &s.root,
        "lhod.epub",
        common::EPUB2_OPF,
        b"<svg xmlns='http://www.w3.org/2000/svg'/>",
    );
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!("not imported");
    };
    let Cover::Undecodable(why) = lib.cover(id).unwrap() else {
        panic!("{:?}", lib.cover(id).unwrap());
    };
    assert!(!why.is_empty());
}

#[test]
fn a_book_with_no_row_gives_unknown_and_the_next_open_fills_it() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!("not imported");
    };
    lib.db.execute("DELETE FROM covers", []).unwrap();
    assert_eq!(lib.cover(id).unwrap(), Cover::Unknown);
    drop(lib);

    let lib = Library::open(&s.config).unwrap();
    let Cover::Image(bytes) = lib.cover(id).unwrap() else {
        panic!("the backfill wrote no image");
    };
    assert_eq!(bytes, cover::thumbnail(common::COVER_JPEG).unwrap());
}

#[test]
fn removing_a_book_deletes_its_cover_row() {
    let s = setup();
    let mut lib = Library::open(&s.config).unwrap();
    let source = common::write_epub(&s.root, "lhod.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&source, false).unwrap() else {
        panic!("not imported");
    };

    lib.remove(id).unwrap();
    assert_eq!(lib.cover(id).unwrap(), Cover::Unknown);
    // The book is gone from `active_books`, so the backfill on the next
    // open writes no row for it.
    drop(lib);
    let lib = Library::open(&s.config).unwrap();
    assert_eq!(lib.cover(id).unwrap(), Cover::Unknown);
}

#[test]
fn a_cover_row_the_table_does_not_allow_is_an_error() {
    let s = setup();
    let lib = Library::open(&s.config).unwrap();
    lib.db
        .execute_batch(
            "INSERT INTO books (id, title) VALUES (1, 'Candide');
             PRAGMA ignore_check_constraints = ON;
             INSERT INTO covers (book_id, state, image) VALUES (1, 'none', x'FFD8FF');
             PRAGMA ignore_check_constraints = OFF;",
        )
        .unwrap();
    let err = lib.cover(1).unwrap_err();
    assert!(err.to_string().contains("none"), "{err}");
}
