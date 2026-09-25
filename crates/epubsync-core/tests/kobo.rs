use epubsync_epub::fixtures as common;

use std::path::Path;

use epubsync_core::device::{Action, Device};
use epubsync_core::kobo::{self, Kobo};
use epubsync_core::library::{ImportOutcome, Library};
use epubsync_core::sync;

/// Makes a folder that looks like a mounted Kobo.
fn fake_kobo(parent: &Path, name: &str, serial: &str) -> std::path::PathBuf {
    let root = parent.join(name);
    std::fs::create_dir_all(root.join(".kobo")).unwrap();
    std::fs::write(
        root.join(".kobo/version"),
        format!("{serial},3.0.35,4.38.23171,3.0.35,3.0.35,00000000-0000-0000-0000-000000000384\n"),
    )
    .unwrap();
    root
}

#[test]
fn detect_finds_kobos_under_the_roots() {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    fake_kobo(&media, "KOBOeReader", "N4181A");
    std::fs::create_dir_all(media.join("USBSTICK")).unwrap();
    let elsewhere = dir.path().join("Volumes");
    fake_kobo(&elsewhere, "KOBO2", "N4181B");

    let found = kobo::detect(&[media, elsewhere, dir.path().join("missing")]);
    let serials: Vec<&str> = found.iter().map(|k| k.serial.as_str()).collect();
    assert_eq!(serials, vec!["N4181A", "N4181B"]);
    assert!(Kobo::at(dir.path()).is_err());
}

#[test]
fn detect_one_wants_exactly_one_kobo() {
    let dir = tempfile::tempdir().unwrap();
    let media = dir.path().join("media");
    let err = kobo::detect_one(std::slice::from_ref(&media))
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "no Kobo found");

    fake_kobo(&media, "KOBOeReader", "N4181A");
    let kobo = kobo::detect_one(std::slice::from_ref(&media)).unwrap();
    assert_eq!(kobo.serial, "N4181A");

    fake_kobo(&media, "KOBO2", "N4181B");
    let err = kobo::detect_one(std::slice::from_ref(&media))
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        format!(
            "more than one Kobo found: {}, {}",
            media.join("KOBO2").display(),
            media.join("KOBOeReader").display()
        )
    );
}

#[test]
fn list_parses_ids_from_file_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_kobo(dir.path(), "KOBOeReader", "N1");
    let kobo = Kobo::at(&root).unwrap();
    assert!(kobo.list().unwrap().is_empty());
    std::fs::create_dir_all(kobo.folder()).unwrap();
    for name in ["3.kepub.epub", "12.kepub.epub", "notes.txt", "x.kepub.epub"] {
        std::fs::write(kobo.folder().join(name), b"").unwrap();
    }
    assert_eq!(
        kobo.list().unwrap().into_iter().collect::<Vec<_>>(),
        vec![3, 12]
    );
    assert_eq!(
        kobo.volume_id(3),
        "file:///mnt/onboard/EpubSync/3.kepub.epub"
    );
}

#[test]
fn remove_dot_underscore_files_keeps_the_books() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_kobo(dir.path(), "KOBOeReader", "N1");
    let kobo = Kobo::at(&root).unwrap();
    assert_eq!(kobo.remove_dot_underscore_files().unwrap(), 0);
    std::fs::create_dir_all(kobo.folder()).unwrap();
    for name in ["3.kepub.epub", "._3.kepub.epub", "._12.kepub.epub"] {
        std::fs::write(kobo.folder().join(name), b"").unwrap();
    }
    assert_eq!(kobo.remove_dot_underscore_files().unwrap(), 2);
    let mut left: Vec<String> = std::fs::read_dir(kobo.folder())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, vec!["3.kepub.epub"]);
}

#[test]
fn sync_copies_replaces_deletes_and_updates_sent() {
    let dir = tempfile::tempdir().unwrap();
    let mut lib = Library::init(&dir.path().join("library")).unwrap();
    let a = common::write_epub(dir.path(), "a.epub", common::EPUB2_OPF);
    let b = common::write_epub(dir.path(), "b.epub", common::BARE_OPF);
    let ImportOutcome::Imported { id: a_id, .. } = lib.import(&a, false).unwrap() else {
        panic!()
    };
    let ImportOutcome::Imported { id: b_id, .. } = lib.import(&b, false).unwrap() else {
        panic!()
    };

    let root = fake_kobo(dir.path(), "KOBOeReader", "N1");
    let mut kobo = Kobo::at(&root).unwrap();
    // A file the library does not know about.
    std::fs::create_dir_all(kobo.folder()).unwrap();
    std::fs::write(kobo.book_path(99), b"stale").unwrap();

    // First sync: send both, delete the stale file.
    let actions = sync::plan(&lib, &kobo).unwrap();
    assert_eq!(
        actions,
        vec![
            Action::Send {
                id: a_id,
                revision: 1
            },
            Action::Send {
                id: b_id,
                revision: 1
            },
            Action::Delete { id: 99 },
        ]
    );
    let mut seen = Vec::new();
    sync::apply(&mut lib, &mut kobo, &actions, |a| seen.push(a.clone())).unwrap();
    assert_eq!(seen, actions);
    assert_eq!(
        std::fs::read(kobo.book_path(a_id)).unwrap(),
        std::fs::read(lib.book_path(a_id)).unwrap()
    );
    assert!(kobo.book_path(b_id).exists());
    assert!(!kobo.book_path(99).exists());
    let sent = |lib: &Library| -> Vec<(i64, i64)> {
        let mut stmt = lib
            .db
            .prepare("SELECT book_id, revision FROM sent ORDER BY book_id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    assert_eq!(sent(&lib), vec![(a_id, 1), (b_id, 1)]);
    let devices: i64 = lib
        .db
        .query_row(
            "SELECT count(*) FROM devices WHERE serial = 'N1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(devices, 1);

    // Second sync: nothing.
    assert_eq!(sync::plan(&lib, &kobo).unwrap(), vec![]);

    // Edit a, delete b on the device, remove nothing: replace and send again.
    let mut record = lib.get(a_id).unwrap().metadata;
    record.title = "Edited".into();
    lib.edit(a_id, &record).unwrap();
    std::fs::remove_file(kobo.book_path(b_id)).unwrap();
    let actions = sync::plan(&lib, &kobo).unwrap();
    assert_eq!(
        actions,
        vec![
            Action::Replace {
                id: a_id,
                revision: 2
            },
            Action::SendAgain {
                id: b_id,
                revision: 1
            }
        ]
    );
    sync::apply(&mut lib, &mut kobo, &actions, |_| {}).unwrap();
    assert_eq!(
        std::fs::read(kobo.book_path(a_id)).unwrap(),
        std::fs::read(lib.book_path(a_id)).unwrap()
    );
    assert!(kobo.book_path(b_id).exists());
    assert_eq!(sent(&lib), vec![(a_id, 2), (b_id, 1)]);

    // Remove a from the library: the next plan deletes it from the device.
    lib.remove(a_id).unwrap();
    let actions = sync::plan(&lib, &kobo).unwrap();
    assert_eq!(actions, vec![Action::Delete { id: a_id }]);
    sync::apply(&mut lib, &mut kobo, &actions, |_| {}).unwrap();
    assert!(!kobo.book_path(a_id).exists());
    assert_eq!(sent(&lib), vec![(b_id, 1)]);

    // Read back is a no-op for now.
    let back = sync::read_back(&mut lib, &mut kobo).unwrap();
    assert!(back.progress.is_empty() && back.words.is_empty());
}
