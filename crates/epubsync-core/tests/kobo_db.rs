use epubsync_epub::fixtures as common;

use std::path::Path;

use epubsync_core::device::{Action, Device, ReadStatus, RowUpdate};
use epubsync_core::kobo::Kobo;
use epubsync_core::kobo::db::{self, KoboDb};
use epubsync_core::library::{ImportOutcome, Library};
use epubsync_core::metadata::{Author, Metadata, Series};
use epubsync_core::sync::{self, Gate};

const SCHEMA: &str = include_str!("fixtures/kobo-schema.sql");

/// Makes a folder that looks like a mounted Kobo, with a database built
/// from the schema fixture at the given version.
fn fake_kobo(parent: &Path, serial: &str, version: i64) -> std::path::PathBuf {
    let root = parent.join("KOBOeReader");
    std::fs::create_dir_all(root.join(".kobo")).unwrap();
    std::fs::write(
        root.join(".kobo/version"),
        format!("{serial},3.0.35,4.38.23171,3.0.35,3.0.35,0\n"),
    )
    .unwrap();
    let conn = rusqlite::Connection::open(root.join(db::DB_PATH)).unwrap();
    conn.execute_batch(SCHEMA).unwrap();
    conn.execute("INSERT INTO dbversion (version) VALUES (?1)", [version])
        .unwrap();
    root
}

fn open_db(root: &Path) -> KoboDb {
    KoboDb::open(&root.join(db::DB_PATH)).unwrap()
}

fn raw(root: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(root.join(db::DB_PATH)).unwrap()
}

/// Inserts a content row the way the firmware does after its scan: 37%
/// read, reading, last read on 1 September, 600 seconds spent.
fn insert_content(root: &Path, volume_id: &str, title: &str, attribution: &str, size: i64) {
    raw(root)
        .execute(
            "INSERT INTO content (ContentID, ContentType, MimeType, ___UserID, Title, Attribution, ___FileSize, ___PercentRead, ReadStatus, DateLastRead, TimeSpentReading, IsDownloaded)
             VALUES (?1, '6', 'application/x-kobo-epub+zip', '', ?2, ?3, ?4, 37, 1, '2026-09-01T10:00:00Z', 600, 'true')",
            rusqlite::params![volume_id, title, attribution, size],
        )
        .unwrap();
}

/// Sets the progress columns of a content row, as reading does.
fn set_progress(
    root: &Path,
    volume_id: &str,
    percent: i64,
    status: i64,
    last_read: Option<&str>,
    time_spent: Option<i64>,
) {
    raw(root)
        .execute(
            "UPDATE content SET ___PercentRead = ?2, ReadStatus = ?3, DateLastRead = ?4, TimeSpentReading = ?5
             WHERE ContentID = ?1",
            rusqlite::params![volume_id, percent, status, last_read, time_spent],
        )
        .unwrap();
}

/// A library's history rows for book 1 on N1: percent, status, last
/// read, time spent, finished at.
type HistoryRows = Vec<(i64, i64, Option<String>, Option<i64>, Option<String>)>;

fn history(lib: &Library) -> HistoryRows {
    let mut stmt = lib
        .db
        .prepare(
            "SELECT percent, status, last_read, time_spent, finished_at FROM progress_history
             WHERE book_id = 1 AND device_serial = 'N1' ORDER BY id",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap();
    rows.collect::<Result<_, _>>().unwrap()
}

const UNTESTED: i64 = 1;
const VOLUME_1: &str = "file:///mnt/onboard/EpubSync/1.kepub.epub";
const STORE_VOLUME: &str = "a1b2c3d4-0000-0000-0000-000000000001";

fn record() -> Metadata {
    Metadata {
        title: "The Left Hand of Darkness".into(),
        authors: vec![
            Author {
                name: "Ursula K. Le Guin".into(),
                sort: "Le Guin, Ursula K.".into(),
            },
            Author {
                name: "Charles Vess".into(),
                sort: "Vess, Charles".into(),
            },
        ],
        series: Some(Series {
            name: "Hainish Cycle".into(),
            number: Some(4.0),
        }),
        publisher: Some("Ace Books".into()),
        description: Some("A novel of Winter.".into()),
    }
}

#[test]
fn the_gate_is_up_on_an_untested_version_and_the_flag_lifts_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    assert!(!open_db(&root).is_tested());
    assert!(!db::TESTED_VERSIONS.contains(&UNTESTED));

    let mut kobo = Kobo::at(&root).unwrap();
    kobo.open_db(false).unwrap();
    assert_eq!(kobo.db_version(), Some(UNTESTED));
    let gate = kobo.write_gate().unwrap();
    assert!(gate.contains("version 1"), "{gate}");
    assert!(gate.contains("--allow-newer-firmware"), "{gate}");
    let Gate::Closed {
        kept,
        skipped,
        reason,
    } = sync::gate(
        vec![
            Action::Send { id: 1, revision: 1 },
            Action::Replace { id: 2, revision: 2 },
            Action::Delete { id: 3 },
        ],
        &kobo,
    )
    else {
        panic!("expected the gate to be closed");
    };
    assert_eq!(
        kept,
        vec![
            Action::Send { id: 1, revision: 1 },
            Action::Delete { id: 3 }
        ]
    );
    assert_eq!(skipped, vec![Action::Replace { id: 2, revision: 2 }]);
    assert!(reason.contains("version 1"), "{reason}");

    kobo.open_db(true).unwrap();
    assert!(kobo.write_gate().is_none());
    if let Some(&tested) = db::TESTED_VERSIONS.first() {
        let root = fake_kobo(&dir.path().join("tested"), "N2", tested);
        assert!(open_db(&root).is_tested());
    }
}

#[test]
fn update_metadata_writes_the_columns_that_differ() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    let db = open_db(&root);
    assert_eq!(
        db.update_metadata(VOLUME_1, &record()).unwrap(),
        RowUpdate::NoRow
    );

    insert_content(
        &root,
        VOLUME_1,
        "The Left Hand of Darkness",
        "Ursula K. Le Guin, Charles Vess",
        1000,
    );
    assert_eq!(
        db.update_metadata(VOLUME_1, &record()).unwrap(),
        RowUpdate::Updated
    );
    let row = db.find_content(VOLUME_1).unwrap().unwrap();
    assert_eq!(row.title.as_deref(), Some("The Left Hand of Darkness"));
    assert_eq!(
        row.attribution.as_deref(),
        Some("Ursula K. Le Guin & Charles Vess")
    );
    assert_eq!(row.series.as_deref(), Some("Hainish Cycle"));
    assert_eq!(row.series_number.as_deref(), Some("4"));
    assert_eq!(row.series_id.as_deref(), Some("Hainish Cycle"));
    assert_eq!(row.series_number_float, Some(4.0));
    assert_eq!(row.description.as_deref(), Some("A novel of Winter."));
    assert_eq!(row.publisher.as_deref(), Some("Ace Books"));
    assert_eq!(row.file_size, 1000);
    assert_eq!(
        db.update_metadata(VOLUME_1, &record()).unwrap(),
        RowUpdate::Unchanged
    );

    let mut cleared = record();
    cleared.series = None;
    assert_eq!(
        db.update_metadata(VOLUME_1, &cleared).unwrap(),
        RowUpdate::Updated
    );
    let row = db.find_content(VOLUME_1).unwrap().unwrap();
    assert!(row.series.is_none() && row.series_id.is_none() && row.series_number_float.is_none());
}

#[test]
fn update_file_size() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    let db = open_db(&root);
    assert!(!db.update_file_size(VOLUME_1, 5).unwrap());
    insert_content(&root, VOLUME_1, "T", "A", 1000);
    assert!(db.update_file_size(VOLUME_1, 2000).unwrap());
    assert_eq!(db.find_content(VOLUME_1).unwrap().unwrap().file_size, 2000);
}

#[test]
fn delete_stale_content_leaves_a_live_row_alone() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    let db = open_db(&root);
    assert!(!db.delete_stale_content(VOLUME_1).unwrap());
    insert_content(&root, VOLUME_1, "T", "A", 1000);
    assert!(!db.delete_stale_content(VOLUME_1).unwrap());
    assert!(db.find_content(VOLUME_1).unwrap().is_some());
    raw(&root)
        .execute(
            "UPDATE content SET IsDownloaded = 'false' WHERE ContentID = ?1",
            [VOLUME_1],
        )
        .unwrap();
    assert!(db.delete_stale_content(VOLUME_1).unwrap());
    assert!(db.find_content(VOLUME_1).unwrap().is_none());
}

#[test]
fn reads_progress() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    let db = open_db(&root);
    assert!(db.progress(VOLUME_1, 1).unwrap().is_none());
    insert_content(&root, VOLUME_1, "T", "A", 1000);
    let p = db.progress(VOLUME_1, 1).unwrap().unwrap();
    assert_eq!(
        (
            p.book_id,
            p.percent,
            p.status,
            p.last_read.as_deref(),
            p.time_spent
        ),
        (
            1,
            37,
            ReadStatus::Reading,
            Some("2026-09-01T10:00:00Z"),
            Some(600)
        )
    );
    // A null reading time reads as none.
    set_progress(&root, VOLUME_1, 37, 1, None, None);
    let p = db.progress(VOLUME_1, 1).unwrap().unwrap();
    assert_eq!((p.last_read, p.time_spent), (None, None));
}

#[test]
fn reads_words_once_and_skips_words_from_other_books() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::init(&dir.path().join("library")).unwrap();
    let epub = common::write_epub(dir.path(), "a.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&epub, false).unwrap() else {
        panic!()
    };
    assert_eq!(id, 1);

    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    insert_content(
        &root,
        VOLUME_1,
        "The Left Hand of Darkness",
        "Ursula K. Le Guin",
        1000,
    );
    insert_content(&root, STORE_VOLUME, "A Store Book", "Someone", 1000);
    let conn = raw(&root);
    for (word, volume, at) in [
        ("ansible", VOLUME_1, "2026-09-02T08:00:00Z"),
        ("kemmer", VOLUME_1, "2026-09-02T09:00:00Z"),
        ("serendipity", STORE_VOLUME, "2026-09-03T10:00:00Z"),
    ] {
        conn.execute(
            "INSERT INTO WordList (Text, VolumeId, DictSuffix, DateCreated) VALUES (?1, ?2, '-en', ?3)",
            rusqlite::params![word, volume, at],
        )
        .unwrap();
    }
    drop(conn);

    let mut kobo = Kobo::at(&root).unwrap();
    kobo.open_db(true).unwrap();
    let back = sync::read_back(&mut lib, &mut kobo).unwrap();
    assert_eq!(back.progress.len(), 1);
    assert_eq!(back.progress[0].percent, 37);
    assert_eq!(back.changed, 1);
    let words: Vec<(&str, i64)> = back
        .words
        .iter()
        .map(|w| (w.word.as_str(), w.book_id))
        .collect();
    assert_eq!(words, [("ansible", 1), ("kemmer", 1)]);

    // A second read adds nothing.
    let back = sync::read_back(&mut lib, &mut kobo).unwrap();
    assert!(back.words.is_empty());
    assert_eq!(back.changed, 0);
    let count: i64 = lib
        .db
        .query_row("SELECT count(*) FROM words", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
    let progress: (i64, i64, String, i64) = lib
        .db
        .query_row("SELECT percent, status, last_read, time_spent FROM progress WHERE book_id = 1 AND device_serial = 'N1'", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })
        .unwrap();
    assert_eq!(progress, (37, 1, "2026-09-01T10:00:00Z".into(), 600));
    assert_eq!(history(&lib).len(), 1);
    kobo.finish().unwrap();
}

#[test]
fn read_back_adds_a_history_row_per_change() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::init(&dir.path().join("library")).unwrap();
    let epub = common::write_epub(dir.path(), "a.epub", common::EPUB2_OPF);
    lib.import(&epub, false).unwrap();
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    insert_content(&root, VOLUME_1, "T", "A", 1000);
    let mut kobo = Kobo::at(&root).unwrap();
    kobo.open_db(true).unwrap();
    let sep_1 = Some("2026-09-01T10:00:00Z");

    // The first read makes one row. The same read makes none.
    assert_eq!(sync::read_back(&mut lib, &mut kobo).unwrap().changed, 1);
    assert_eq!(sync::read_back(&mut lib, &mut kobo).unwrap().changed, 0);
    // Each column adds a row on its own.
    set_progress(&root, VOLUME_1, 40, 1, sep_1, Some(600));
    assert_eq!(sync::read_back(&mut lib, &mut kobo).unwrap().changed, 1);
    set_progress(&root, VOLUME_1, 40, 0, sep_1, Some(600));
    assert_eq!(sync::read_back(&mut lib, &mut kobo).unwrap().changed, 1);
    set_progress(
        &root,
        VOLUME_1,
        40,
        0,
        Some("2026-09-02T10:00:00Z"),
        Some(600),
    );
    assert_eq!(sync::read_back(&mut lib, &mut kobo).unwrap().changed, 1);
    set_progress(
        &root,
        VOLUME_1,
        40,
        0,
        Some("2026-09-02T10:00:00Z"),
        Some(900),
    );
    assert_eq!(sync::read_back(&mut lib, &mut kobo).unwrap().changed, 1);
    let rows = history(&lib);
    assert_eq!(rows.len(), 5);
    assert_eq!(
        rows[4],
        (40, 0, Some("2026-09-02T10:00:00Z".into()), Some(900), None)
    );
    assert!(rows.iter().all(|r| r.4.is_none()));
    kobo.finish().unwrap();
}

#[test]
fn read_back_sets_the_finished_date_when_the_status_turns_finished() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::init(&dir.path().join("library")).unwrap();
    let epub = common::write_epub(dir.path(), "a.epub", common::EPUB2_OPF);
    lib.import(&epub, false).unwrap();
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    insert_content(&root, VOLUME_1, "T", "A", 1000);
    let mut kobo = Kobo::at(&root).unwrap();
    kobo.open_db(true).unwrap();
    sync::read_back(&mut lib, &mut kobo).unwrap();

    // Finished: the date is the last read time.
    set_progress(
        &root,
        VOLUME_1,
        100,
        2,
        Some("2026-09-03T10:00:00Z"),
        Some(700),
    );
    sync::read_back(&mut lib, &mut kobo).unwrap();
    // Opened again: the date stays.
    set_progress(
        &root,
        VOLUME_1,
        10,
        1,
        Some("2026-09-10T10:00:00Z"),
        Some(800),
    );
    sync::read_back(&mut lib, &mut kobo).unwrap();
    // Finished again: the date moves.
    set_progress(
        &root,
        VOLUME_1,
        100,
        2,
        Some("2026-09-20T10:00:00Z"),
        Some(900),
    );
    sync::read_back(&mut lib, &mut kobo).unwrap();
    let dates: Vec<Option<String>> = history(&lib).into_iter().map(|r| r.4).collect();
    assert_eq!(
        dates,
        [
            None,
            Some("2026-09-03T10:00:00Z".into()),
            Some("2026-09-03T10:00:00Z".into()),
            Some("2026-09-20T10:00:00Z".into()),
        ]
    );

    // A row with no last read time that turns finished gets the sync time.
    let root_2 = fake_kobo(&dir.path().join("second"), "N2", UNTESTED);
    insert_content(&root_2, VOLUME_1, "T", "A", 1000);
    set_progress(&root_2, VOLUME_1, 100, 2, None, None);
    let mut kobo_2 = Kobo::at(&root_2).unwrap();
    kobo_2.open_db(true).unwrap();
    sync::read_back(&mut lib, &mut kobo_2).unwrap();
    let (last_read, finished_at, seen_at): (Option<String>, String, String) = lib
        .db
        .query_row(
            "SELECT last_read, finished_at, seen_at FROM progress_history ORDER BY id DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(last_read, None);
    assert_eq!(finished_at, seen_at);
    assert_eq!(seen_at.len(), "2026-09-19T12:00:00Z".len());
    kobo.finish().unwrap();
    kobo_2.finish().unwrap();
}

#[test]
fn apply_updates_the_file_size_on_replace_and_sends_again_after_a_device_delete() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::init(&dir.path().join("library")).unwrap();
    let epub = common::write_epub(dir.path(), "a.epub", common::EPUB2_OPF);
    let ImportOutcome::Imported { id, .. } = lib.import(&epub, false).unwrap() else {
        panic!()
    };
    let root = fake_kobo(dir.path(), "N1", UNTESTED);
    let mut kobo = Kobo::at(&root).unwrap();
    kobo.open_db(true).unwrap();

    let actions = sync::plan(&lib, &kobo).unwrap();
    sync::apply(&mut lib, &mut kobo, &actions, |_| {}).unwrap();
    // No row yet: the firmware makes it on its scan.
    assert_eq!(
        sync::update_rows(&lib, &mut kobo).unwrap(),
        vec![(id, RowUpdate::NoRow)]
    );
    insert_content(&root, VOLUME_1, "old title", "old", 1);
    assert_eq!(
        sync::update_rows(&lib, &mut kobo).unwrap(),
        vec![(id, RowUpdate::Updated)]
    );

    // Replace: the stored size follows the new file.
    let mut record = lib.get(id).unwrap().metadata;
    record.title = "Edited".into();
    lib.edit(id, &record).unwrap();
    let actions = sync::plan(&lib, &kobo).unwrap();
    assert_eq!(actions, vec![Action::Replace { id, revision: 2 }]);
    sync::apply(&mut lib, &mut kobo, &actions, |_| {}).unwrap();
    let size = std::fs::metadata(lib.book_path(id)).unwrap().len() as i64;
    assert_eq!(
        open_db(&root)
            .find_content(VOLUME_1)
            .unwrap()
            .unwrap()
            .file_size,
        size
    );

    // Deleted on the device: the firmware drops the row itself, and the
    // next sync sends the book again.
    std::fs::remove_file(kobo.book_path(id)).unwrap();
    raw(&root)
        .execute("DELETE FROM content WHERE ContentID = ?1", [VOLUME_1])
        .unwrap();
    let actions = sync::plan(&lib, &kobo).unwrap();
    assert_eq!(actions, vec![Action::SendAgain { id, revision: 2 }]);
    sync::apply(&mut lib, &mut kobo, &actions, |_| {}).unwrap();
    assert!(kobo.book_path(id).exists());

    // Deleted on the device with the row left behind: sync deletes the
    // row before it copies the file.
    std::fs::remove_file(kobo.book_path(id)).unwrap();
    insert_content(&root, VOLUME_1, "T", "A", 1);
    raw(&root)
        .execute(
            "UPDATE content SET IsDownloaded = 'false' WHERE ContentID = ?1",
            [VOLUME_1],
        )
        .unwrap();
    let actions = sync::plan(&lib, &kobo).unwrap();
    assert_eq!(actions, vec![Action::SendAgain { id, revision: 2 }]);
    sync::apply(&mut lib, &mut kobo, &actions, |_| {}).unwrap();
    assert!(kobo.book_path(id).exists());
    assert!(open_db(&root).find_content(VOLUME_1).unwrap().is_none());

    // With the gate up, a replace leaves the row alone.
    kobo.open_db(false).unwrap();
    insert_content(&root, VOLUME_1, "T", "A", 1);
    let mut record = lib.get(id).unwrap().metadata;
    record.title = "Edited again".into();
    lib.edit(id, &record).unwrap();
    let Gate::Closed { kept, skipped, .. } = sync::gate(sync::plan(&lib, &kobo).unwrap(), &kobo)
    else {
        panic!("expected the gate to be closed");
    };
    assert!(kept.is_empty());
    assert_eq!(skipped, vec![Action::Replace { id, revision: 3 }]);
    kobo.finish().unwrap();
}
