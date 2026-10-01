//! Runs the binary against a temp library.

use std::io::Write;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

const OPF: &str = r##"<?xml version='1.0' encoding='utf-8'?>
<package xmlns="http://www.idpf.org/2007/opf" unique-identifier="uuid_id" version="2.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:opf="http://www.idpf.org/2007/opf">
    <dc:identifier id="uuid_id" opf:scheme="uuid">a1b2c3</dc:identifier>
    <dc:title>The Left Hand of Darkness</dc:title>
    <dc:creator opf:role="aut">Ursula K. Le Guin</dc:creator>
    <dc:language>en</dc:language>
    <meta name="calibre:series" content="Hainish Cycle"/>
    <meta name="calibre:series_index" content="4"/>
  </metadata>
  <manifest>
    <item id="ch1" href="chapter1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>
"##;

fn write_epub(dir: &Path, name: &str, opf: &str) -> PathBuf {
    use zip::CompressionMethod;
    use zip::write::SimpleFileOptions;
    let path = dir.join(name);
    let file = std::fs::File::create(&path).unwrap();
    let mut zw = zip::ZipWriter::new(file);
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zw.start_file("mimetype", stored).unwrap();
    zw.write_all(b"application/epub+zip").unwrap();
    zw.start_file("META-INF/container.xml", deflated).unwrap();
    zw.write_all(br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#).unwrap();
    zw.start_file("OEBPS/content.opf", deflated).unwrap();
    zw.write_all(opf.as_bytes()).unwrap();
    zw.start_file("OEBPS/chapter1.xhtml", deflated).unwrap();
    zw.write_all(br#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><head><title>1</title></head><body><p>Hello.</p></body></html>"#).unwrap();
    zw.finish().unwrap();
    path
}

struct Env {
    /// Deletes the temp folder when the test ends.
    _dir: tempfile::TempDir,
    /// The temp folder with symlinks resolved. The library stores its
    /// folder canonicalized, so a path the CLI prints matches a path built
    /// from this one. On macOS the plain temp path starts with `/tmp`,
    /// a symlink to `/private/tmp`, and would not match.
    root: PathBuf,
}

impl Env {
    fn new() -> Env {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        Env { _dir: dir, root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    fn cmd(&self) -> Command {
        let mut cmd = Command::cargo_bin("epubsync").unwrap();
        cmd.env("EPUBSYNC_CONFIG", self.path().join("config.toml"));
        cmd.env_remove("EDITOR");
        cmd
    }

    fn init(&self) {
        self.cmd()
            .args(["init", self.path().join("library").to_str().unwrap()])
            .assert()
            .success();
    }
}

/// Runs a command that must succeed and parses its stdout as JSON.
fn json_output(cmd: &mut Command) -> serde_json::Value {
    let out = cmd.assert().success().get_output().stdout.clone();
    serde_json::from_slice(&out).unwrap_or_else(|e| {
        panic!("{e}:\n{}", String::from_utf8_lossy(&out));
    })
}

#[test]
fn every_command_but_init_needs_the_config() {
    let env = Env::new();
    env.cmd()
        .arg("list")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Run `epubsync init <folder>`"));
}

#[test]
fn init_import_list_edit_remove() {
    let env = Env::new();
    env.init();
    assert!(env.path().join("library/library.sqlite").exists());

    let epub = write_epub(env.path(), "lhod.epub", OPF);
    env.cmd()
        .args(["import", epub.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("    1  The Left Hand of Darkness"))
        .stdout(predicate::str::contains(
            "made sort name for Ursula K. Le Guin: Le Guin, Ursula K.",
        ));
    assert!(env.path().join("library/1.kepub.epub").exists());

    env.cmd()
        .args(["import", epub.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("already in the library, skipped"));

    env.cmd()
        .arg("list")
        .assert()
        .success()
        .stdout("    1  The Left Hand of Darkness  by Ursula K. Le Guin  [Hainish Cycle #4]\n");

    env.cmd()
        .args([
            "edit",
            "1",
            "--title",
            "The Left Hand",
            "--series-number",
            "4.5",
            "--author",
            "Ursula K. Le Guin|Le Guin, Ursula",
        ])
        .assert()
        .success()
        .stdout("    1  The Left Hand  by Ursula K. Le Guin  [Hainish Cycle #4.5]\n");

    env.cmd()
        .args(["edit", "1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("set $EDITOR"));

    env.cmd()
        .args(["remove", "1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--yes"));
    assert!(env.path().join("library/1.kepub.epub").exists());

    env.cmd()
        .args(["remove", "1", "--yes"])
        .assert()
        .success()
        .stdout("removed 1 \"The Left Hand\"\n");
    assert!(!env.path().join("library/1.kepub.epub").exists());
    env.cmd().arg("list").assert().success().stdout("");
}

#[test]
fn imports_a_folder() {
    let env = Env::new();
    env.init();
    let books = env.path().join("books");
    std::fs::create_dir(&books).unwrap();
    write_epub(&books, "a.epub", OPF);
    write_epub(
        &books,
        "b.epub",
        &OPF.replace("The Left Hand of Darkness", "The Dispossessed"),
    );
    std::fs::write(books.join("notes.txt"), "not a book").unwrap();
    env.cmd()
        .args(["import", books.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("    1  The Left Hand of Darkness"))
        .stdout(predicate::str::contains("    2  The Dispossessed"));
}

#[test]
fn edits_through_the_editor() {
    let env = Env::new();
    env.init();
    let epub = write_epub(env.path(), "lhod.epub", OPF);
    env.cmd()
        .args(["import", epub.to_str().unwrap()])
        .assert()
        .success();

    // An "editor" that rewrites the title with sed.
    let editor = env.path().join("editor.sh");
    std::fs::write(
        &editor,
        "#!/bin/sh\nsed -i 's/The Left Hand of Darkness/Winter/' \"$1\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&editor, std::fs::Permissions::from_mode(0o755)).unwrap();

    env.cmd()
        .env("EDITOR", editor.to_str().unwrap())
        .args(["edit", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "    1  Winter  by Ursula K. Le Guin",
        ));
}

#[test]
fn syncs_to_a_folder_that_looks_like_a_kobo() {
    let env = Env::new();
    env.init();
    let epub = write_epub(env.path(), "lhod.epub", OPF);
    env.cmd()
        .args(["import", epub.to_str().unwrap()])
        .assert()
        .success();

    let kobo = env.path().join("KOBOeReader");
    std::fs::create_dir_all(kobo.join(".kobo")).unwrap();
    std::fs::write(
        kobo.join(".kobo/version"),
        "N4181A,3.0.35,4.38.23171,3.0.35,3.0.35,0\n",
    )
    .unwrap();
    std::fs::create_dir_all(kobo.join("EpubSync")).unwrap();
    std::fs::write(kobo.join("EpubSync/42.kepub.epub"), b"stale").unwrap();

    env.cmd()
        .args(["sync", "--device", kobo.to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Kobo N4181A at"))
        .stdout(predicate::str::contains(
            "send            1  The Left Hand of Darkness",
        ))
        .stdout(predicate::str::contains(
            "delete         42  (no longer in the library)",
        ))
        .stdout(predicate::str::contains("eject").not());
    assert!(!kobo.join("EpubSync/1.kepub.epub").exists());

    let plan = json_output(env.cmd().args([
        "sync",
        "--device",
        kobo.to_str().unwrap(),
        "--dry-run",
        "--json",
    ]));
    assert_eq!(
        plan,
        serde_json::json!({
            "device": {"serial": "N4181A", "root": kobo.to_str().unwrap(), "db_version": null},
            "write_gate": null,
            "actions": [
                {"action": "send", "id": 1, "revision": 1, "title": "The Left Hand of Darkness"},
                {"action": "delete", "id": 42},
            ],
            "skipped": [],
        })
    );
    assert!(!kobo.join("EpubSync/1.kepub.epub").exists());

    env.cmd()
        .args(["sync", "--device", kobo.to_str().unwrap(), "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--dry-run"));

    env.cmd()
        .args(["sync", "--device", kobo.to_str().unwrap()])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--yes"));

    env.cmd()
        .args(["sync", "--device", kobo.to_str().unwrap(), "--yes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sending 1"))
        .stdout(predicate::str::contains("deleting 42"))
        .stdout(predicate::str::contains(
            "run `epubsync eject` before you unplug the device",
        ));
    assert!(kobo.join("EpubSync/1.kepub.epub").exists());
    assert!(!kobo.join("EpubSync/42.kepub.epub").exists());

    env.cmd()
        .args(["sync", "--device", kobo.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("nothing to do"));
}

#[test]
fn shows_one_book() {
    let env = Env::new();
    env.init();
    let opf = OPF.replace(
        "<dc:language>en</dc:language>",
        "<dc:language>en</dc:language>
    <dc:publisher>Ace</dc:publisher>
    <dc:description>&lt;p&gt;A &lt;em&gt;human&lt;/em&gt; envoy.&lt;/p&gt;</dc:description>",
    );
    let epub = write_epub(env.path(), "lhod.epub", &opf);
    env.cmd()
        .args(["import", epub.to_str().unwrap()])
        .assert()
        .success();

    env.cmd()
        .args(["show", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Device     not yet sent to a device\n",
        ));

    let db = rusqlite::Connection::open(env.path().join("library/library.sqlite")).unwrap();
    db.execute_batch(
        "INSERT INTO devices (serial) VALUES ('N1');
         INSERT INTO progress_history (book_id, device_serial, percent, status, last_read, time_spent, finished_at, seen_at) VALUES
           (1, 'N1', 20, 1, '2026-08-20T10:00:00Z', 600, NULL, '2026-08-21T09:00:00Z'),
           (1, 'N1', 37, 1, '2026-09-01T10:00:00Z', 12000, NULL, '2026-09-02T09:00:00Z');",
    )
    .unwrap();
    drop(db);

    let out = env
        .cmd()
        .args(["show", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let file = env.path().join("library/1.kepub.epub");
    for line in [
        "Id         1\n",
        "Title      The Left Hand of Darkness\n",
        "Author     Ursula K. Le Guin (sort: Le Guin, Ursula K.)\n",
        "Publisher  Ace\nSeries     Hainish Cycle, book 4\nWords      1\n",
        "Ease       ",
        "Revision   1\n",
        &format!("File       {}\n", file.display()),
        "Device     N1: 37% reading 2026-09-01, 3 h 20 min\n",
        "History    2026-08-21  N1: 20% reading 2026-08-20\nHistory    2026-09-02  N1: 37% reading 2026-09-01\n",
        "\nA *human* envoy.\n",
    ] {
        assert!(out.contains(line), "{line:?} not in:\n{out}");
    }

    let mut book = json_output(env.cmd().args(["show", "1", "--json"]));
    // The score depends on the scorer, so the test checks only that it is a number.
    let ease = book.as_object_mut().unwrap().remove("reading_ease");
    assert!(ease.as_ref().is_some_and(|e| e.is_number()), "{ease:?}");
    assert_eq!(
        book,
        serde_json::json!({
            "id": 1,
            "revision": 1,
            "title": "The Left Hand of Darkness",
            "authors": [{"name": "Ursula K. Le Guin", "sort": "Le Guin, Ursula K."}],
            "series": {"name": "Hainish Cycle", "number": 4.0},
            "publisher": "Ace",
            "description": "<p>A <em>human</em> envoy.</p>",
            "word_count": 1,
            "file": file.to_str().unwrap(),
            "progress": [
                {"device_serial": "N1", "percent": 37, "status": "reading", "last_read": "2026-09-01T10:00:00Z",
                 "time_spent": 12000, "finished_at": null},
            ],
            "history": [
                {"device_serial": "N1", "percent": 20, "status": "reading", "last_read": "2026-08-20T10:00:00Z",
                 "time_spent": 600, "finished_at": null, "seen_at": "2026-08-21T09:00:00Z"},
                {"device_serial": "N1", "percent": 37, "status": "reading", "last_read": "2026-09-01T10:00:00Z",
                 "time_spent": 12000, "finished_at": null, "seen_at": "2026-09-02T09:00:00Z"},
            ],
        })
    );

    env.cmd()
        .args(["show", "2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no book with id 2"));
}

#[test]
fn open_fails_on_an_unknown_id() {
    let env = Env::new();
    env.init();
    env.cmd()
        .args(["open", "1"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no book with id 1"));
}

#[test]
fn lists_progress_and_words() {
    let env = Env::new();
    env.init();
    let epub = write_epub(env.path(), "lhod.epub", OPF);
    env.cmd()
        .args(["import", epub.to_str().unwrap()])
        .assert()
        .success();

    let db = rusqlite::Connection::open(env.path().join("library/library.sqlite")).unwrap();
    db.execute_batch(
        "INSERT INTO devices (serial) VALUES ('N1'), ('N2');
         INSERT INTO progress_history (book_id, device_serial, percent, status, last_read, time_spent, finished_at, seen_at) VALUES
           (1, 'N1', 37, 1, '2026-09-01T10:00:00Z', 1200, NULL, '2026-09-02T09:00:00Z'),
           (1, 'N2', 100, 2, '2026-08-01T10:00:00Z', NULL, '2026-08-01T10:00:00Z', '2026-08-02T09:00:00Z');
         INSERT INTO books (id, title, deleted_at) VALUES (2, 'A Removed Book', '2026-09-03T00:00:00Z');
         INSERT INTO words (word, device_serial, book_id, dict_suffix, looked_up_at) VALUES
           ('ansible', 'N1', 1, '-en', '2026-09-02T08:00:00Z'),
           ('kemmer', 'N1', 1, '-en', '2026-09-02T09:00:00Z'),
           ('serendipity', 'N2', 2, '-en', '2026-09-03T10:00:00Z');",
    )
    .unwrap();
    drop(db);

    env.cmd().arg("list").assert().success().stdout(
        "    1  The Left Hand of Darkness  by Ursula K. Le Guin  [Hainish Cycle #4]  N1: 37% reading 2026-09-01, 20 min  N2: 100% finished 2026-08-01\n",
    );

    // The same data as JSON: one flat object per book, with the fields a
    // book does not have left out, the progress rows in device order, and
    // no history.
    let mut books = json_output(env.cmd().args(["list", "--json"]));
    let book = &mut books[0];
    assert!(
        book.as_object_mut()
            .unwrap()
            .remove("reading_ease")
            .is_some()
    );
    assert_eq!(
        books,
        serde_json::json!([{
            "id": 1,
            "revision": 1,
            "title": "The Left Hand of Darkness",
            "authors": [{"name": "Ursula K. Le Guin", "sort": "Le Guin, Ursula K."}],
            "series": {"name": "Hainish Cycle", "number": 4.0},
            "word_count": 1,
            "file": env.path().join("library/1.kepub.epub").to_str().unwrap(),
            "progress": [
                {"device_serial": "N1", "percent": 37, "status": "reading", "last_read": "2026-09-01T10:00:00Z",
                 "time_spent": 1200, "finished_at": null},
                {"device_serial": "N2", "percent": 100, "status": "finished", "last_read": "2026-08-01T10:00:00Z",
                 "time_spent": null, "finished_at": "2026-08-01T10:00:00Z"},
            ],
        }])
    );
    assert_eq!(
        json_output(env.cmd().args(["list", "--json", "tolkien"])),
        serde_json::json!([])
    );

    let words = json_output(env.cmd().args(["words", "--json"]));
    assert_eq!(
        words,
        serde_json::json!([
            {"word": "serendipity", "device_serial": "N2", "book_id": 2,
             "book_title": "A Removed Book", "dict_suffix": "-en", "looked_up_at": "2026-09-03T10:00:00Z"},
            {"word": "kemmer", "device_serial": "N1", "book_id": 1,
             "book_title": "The Left Hand of Darkness", "dict_suffix": "-en", "looked_up_at": "2026-09-02T09:00:00Z"},
            {"word": "ansible", "device_serial": "N1", "book_id": 1,
             "book_title": "The Left Hand of Darkness", "dict_suffix": "-en", "looked_up_at": "2026-09-02T08:00:00Z"},
        ])
    );
    assert_eq!(
        json_output(env.cmd().args(["words", "--json", "--device", "N2"]))
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let out = env
        .cmd()
        .arg("words")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines.len(), 3);
    assert!(
        lines[0].starts_with("2026-09-03T10:00:00Z  serendipity"),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].contains("    2  A Removed Book  (N2)"),
        "{}",
        lines[0]
    );
    assert!(
        lines[1].starts_with("2026-09-02T09:00:00Z  kemmer"),
        "{}",
        lines[1]
    );
    assert!(
        lines[1].contains("    1  The Left Hand of Darkness  (N1)"),
        "{}",
        lines[1]
    );
    assert!(
        lines[2].starts_with("2026-09-02T08:00:00Z  ansible"),
        "{}",
        lines[2]
    );

    env.cmd()
        .args(["words", "--book", "1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("serendipity").not())
        .stdout(predicate::str::contains("ansible"));
    env.cmd()
        .args(["words", "--device", "N2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("serendipity"))
        .stdout(predicate::str::contains("ansible").not());
}

/// Three books: The Left Hand of Darkness (Le Guin, Hainish Cycle 4,
/// finished on N1 in August), A Wizard of Earthsea (Le Guin, Earthsea 1,
/// finished on N1 in July and being read again in September), and Dune
/// (Herbert, no series, not sent).
fn library_of_three() -> Env {
    let env = Env::new();
    env.init();
    let books = env.path().join("books");
    std::fs::create_dir(&books).unwrap();
    write_epub(&books, "a.epub", OPF);
    write_epub(
        &books,
        "b.epub",
        &OPF.replace("The Left Hand of Darkness", "A Wizard of Earthsea")
            .replace("Hainish Cycle", "Earthsea")
            .replace("content=\"4\"", "content=\"1\""),
    );
    write_epub(
        &books,
        "c.epub",
        &OPF.replace("The Left Hand of Darkness", "Dune")
            .replace("Ursula K. Le Guin", "Frank Herbert")
            .replace(
                "    <meta name=\"calibre:series\" content=\"Hainish Cycle\"/>\n    <meta name=\"calibre:series_index\" content=\"4\"/>\n",
                "",
            ),
    );
    env.cmd()
        .args(["import", books.to_str().unwrap()])
        .assert()
        .success();
    let db = rusqlite::Connection::open(env.path().join("library/library.sqlite")).unwrap();
    db.execute_batch(
        "INSERT INTO devices (serial) VALUES ('N1');
         INSERT INTO progress_history (book_id, device_serial, percent, status, last_read, finished_at, seen_at) VALUES
           (1, 'N1', 100, 2, '2026-08-01T10:00:00Z', '2026-08-01T10:00:00Z', '2026-08-02T09:00:00Z'),
           (2, 'N1', 100, 2, '2026-07-01T10:00:00Z', '2026-07-01T10:00:00Z', '2026-07-02T09:00:00Z'),
           (2, 'N1', 37, 1, '2026-09-01T10:00:00Z', '2026-07-01T10:00:00Z', '2026-09-02T09:00:00Z');",
    )
    .unwrap();
    env
}

#[test]
fn list_filters_by_text_author_series_and_status() {
    let env = library_of_three();
    let titles = |args: &[&str]| -> Vec<String> {
        let out = env
            .cmd()
            .arg("list")
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| l[7..].split("  ").next().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        titles(&[]),
        ["The Left Hand of Darkness", "A Wizard of Earthsea", "Dune"]
    );
    assert_eq!(titles(&["earth"]), ["A Wizard of Earthsea"]);
    assert_eq!(titles(&["herbert"]), ["Dune"]);
    assert_eq!(
        titles(&["--author", "le guin"]),
        ["The Left Hand of Darkness", "A Wizard of Earthsea"]
    );
    assert_eq!(
        titles(&["--title", "of"]),
        ["The Left Hand of Darkness", "A Wizard of Earthsea"]
    );
    assert_eq!(
        titles(&["--series", "hainish"]),
        ["The Left Hand of Darkness"]
    );
    assert_eq!(
        titles(&["--author", "le guin", "--series", "earthsea"]),
        ["A Wizard of Earthsea"]
    );
    assert_eq!(titles(&["--reading"]), ["A Wizard of Earthsea"]);
    assert_eq!(titles(&["--finished"]), ["The Left Hand of Darkness"]);
    assert_eq!(titles(&["--unread"]), ["Dune"]);
    assert!(titles(&["--author", "tolkien"]).is_empty());

    env.cmd()
        .args(["list", "--reading", "--finished"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn list_sorts_by_a_key_and_reverses() {
    let env = library_of_three();
    let ids = |args: &[&str]| -> Vec<i64> {
        let out = env
            .cmd()
            .arg("list")
            .args(args)
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| l.trim_start().split("  ").next().unwrap().parse().unwrap())
            .collect()
    };
    assert_eq!(ids(&[]), [1, 2, 3]);
    // Dune, Left Hand of Darkness, Wizard of Earthsea: the articles do not count.
    assert_eq!(ids(&["--sort", "title"]), [3, 1, 2]);
    assert_eq!(ids(&["--sort", "title", "--reverse"]), [2, 1, 3]);
    // Herbert, then Le Guin twice in id order.
    assert_eq!(ids(&["--sort", "author"]), [3, 1, 2]);
    // Earthsea, Hainish Cycle, then the book with no series.
    assert_eq!(ids(&["--sort", "series"]), [2, 1, 3]);
    // 37%, 100%, then the book with no progress row.
    assert_eq!(ids(&["--sort", "progress"]), [2, 1, 3]);
    // August, September, then the book never read.
    assert_eq!(ids(&["--sort", "last-read"]), [1, 2, 3]);
    assert_eq!(ids(&["--sort", "last-read", "--reverse"]), [3, 2, 1]);
    // July, August, then the book never finished. A Wizard of Earthsea
    // keeps its date while it is read again.
    assert_eq!(ids(&["--sort", "finished"]), [2, 1, 3]);
    assert_eq!(ids(&["--sort", "finished", "--reverse"]), [3, 1, 2]);
    // Herbert, then Le Guin's books by series: Earthsea before Hainish Cycle.
    assert_eq!(ids(&["--sort", "author,series"]), [3, 2, 1]);
    assert_eq!(ids(&["--sort", "author", "--sort", "series"]), [3, 2, 1]);

    env.cmd()
        .args(["list", "--sort", "colour"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("last-read"));
}

#[test]
fn eject_skips_a_folder_that_is_not_a_volume() {
    let env = Env::new();
    env.init();
    let kobo = env.path().join("KOBOeReader");
    std::fs::create_dir_all(kobo.join(".kobo")).unwrap();
    std::fs::write(
        kobo.join(".kobo/version"),
        "N4181A,3.0.35,4.38.23171,3.0.35,3.0.35,0\n",
    )
    .unwrap();
    env.cmd()
        .args(["eject", "--device", kobo.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "not a mounted volume; nothing to eject",
        ));
}
