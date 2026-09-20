//! The library folder: the book files, the SQLite database, and the lock.
//! Every command opens the library once and holds the lock until it ends.

use std::collections::BTreeMap;
use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use rusqlite_migration::{M, Migrations};
use serde::Serialize;

use crate::config::Config;
use crate::cover::Cover;
use crate::device::ReadStatus;
use crate::metadata::{Author, Metadata, Series};
use crate::sort_name::sort_name;
use crate::stats::Engines;
pub use crate::stats::Stats;
use crate::{kepub, stats};
use epubsync_epub::Epub;

const TABLES_SQL: &str = include_str!("migrations/1-tables.sql");
const BOOKS_AUTOINCREMENT_SQL: &str = include_str!("migrations/2-books-autoincrement.sql");
const DELETED_BOOKS_SQL: &str = include_str!("migrations/3-deleted-books.sql");
const PROGRESS_HISTORY_SQL: &str = include_str!("migrations/4-progress-history.sql");
const COVERS_SQL: &str = include_str!("migrations/5-covers.sql");
const DB_NAME: &str = "library.sqlite";
const LOCK_NAME: &str = "lock";

pub struct Library {
    pub folder: PathBuf,
    pub db: Connection,
    /// The exclusive lock on the lock file. It is released when the
    /// Library drops, at the end of the command.
    _lock: File,
}

/// A book as the library holds it. As JSON it is one flat object: the
/// metadata and the stats fields sit next to `id` and `revision`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Book {
    pub id: i64,
    pub revision: i64,
    #[serde(flatten)]
    pub metadata: Metadata,
    #[serde(flatten)]
    pub stats: Stats,
}

/// One optional detail of a book, with its value as the library stores
/// it. A display picks the label and the text for each one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Field<'a> {
    Publisher(&'a str),
    Series(&'a Series),
    WordCount(u64),
    ReadingEase(f64),
}

impl Book {
    /// The optional details the book has, in the order a display lists
    /// them. A detail the book does not have gets no entry.
    pub fn fields(&self) -> Vec<Field<'_>> {
        let m = &self.metadata;
        [
            m.publisher.as_deref().map(Field::Publisher),
            m.series.as_ref().map(Field::Series),
            self.stats.word_count.map(Field::WordCount),
            self.stats.reading_ease.map(Field::ReadingEase),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ImportOutcome {
    /// The file is in the library. `made_sort` lists the authors that got
    /// a sort name made from the display name.
    Imported { id: i64, made_sort: Vec<Author> },
    /// A book with the same title and first author is already in the
    /// library, and `force` was not given.
    Exists { id: i64 },
}

impl Library {
    /// Creates the folder and the database.
    pub fn init(folder: &Path) -> Result<Library> {
        std::fs::create_dir_all(folder).with_context(|| format!("create {}", folder.display()))?;
        let folder = folder.canonicalize()?;
        if folder.join(DB_NAME).exists() {
            bail!("{} already holds a library", folder.display());
        }
        let config = Config { library: folder };
        let lib = Library::open(&config)?;
        Ok(lib)
    }

    /// Opens the library named by the config, taking the exclusive lock,
    /// and runs the migrations the database is missing. A missing
    /// database file is created and gets every migration.
    pub fn open(config: &Config) -> Result<Library> {
        let folder = config.library.clone();
        let lock_path = folder.join(LOCK_NAME);
        let lock_file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("open {}", lock_path.display()))?;
        match lock_file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                bail!("another EpubSync is running on {}", folder.display())
            }
            Err(TryLockError::Error(e)) => {
                return Err(e).with_context(|| format!("lock {}", lock_path.display()));
            }
        }
        let db_path = folder.join(DB_NAME);
        let mut db =
            Connection::open(&db_path).with_context(|| format!("open {}", db_path.display()))?;
        // Foreign keys are off while the migrations run, as SQLite advises
        // for schema changes, and on after. The bundled SQLite turns them
        // on by default, so they are turned off here first. A migration
        // that rebuilds a table runs `foreign_key_check` at its end.
        db.execute_batch("PRAGMA foreign_keys = OFF;")?;
        migrations()
            .to_latest(&mut db)
            .context("migrate the database")?;
        db.execute_batch("PRAGMA foreign_keys = ON;")?;
        let library = Library {
            folder,
            db,
            _lock: lock_file,
        };
        library.fill_covers()?;
        Ok(library)
    }

    /// The path of a book's file in the library folder.
    pub fn book_path(&self, id: i64) -> PathBuf {
        self.folder.join(book_file_name(id))
    }

    /// Imports an EPUB or KEPUB. See the design for the six steps.
    pub fn import(&mut self, source: &Path, force: bool) -> Result<ImportOutcome> {
        // 1 and 2: read the file metadata, measure a file that carries no
        // word count, and make missing sort names.
        let source_epub = Epub::open(source)?;
        let (record, made_sort) = source_epub.metadata(sort_name);
        let file_stats = source_epub.stats();
        let measured = file_stats.word_count.is_none();
        let stats = if measured {
            stats::measure(&source_epub, &mut Engines::default())
                .with_context(|| format!("measure {}", source.display()))?
        } else {
            file_stats
        };

        // 3: stop on a book with the same title and first author.
        if !force && let Some(id) = self.find_same(&record)? {
            return Ok(ImportOutcome::Exists { id });
        }

        // 4: convert or copy into a temp file in the library folder. The
        // file is removed with the value when a later step fails, so an
        // import that returns an error leaves nothing in the folder.
        let temp = tempfile::Builder::new()
            .prefix("import-")
            .suffix(".tmp")
            .tempfile_in(&self.folder)
            .context("create a temp file in the library")?;
        let is_kepub = source
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".kepub.epub") || n.ends_with(".kepub"));
        if is_kepub {
            std::fs::copy(source, temp.path())
                .with_context(|| format!("copy {}", source.display()))?;
        } else {
            kepub::convert(source, temp.path())
                .with_context(|| format!("convert {}", source.display()))?;
        }

        // 5: insert the row and rename the file to its id.
        let id = self.insert(&record, &stats)?;
        temp.persist(self.book_path(id))
            .map_err(|e| e.error)
            .context("rename the imported file")?;

        // 6: write made sort names and measured numbers into the converted
        // file.
        if !made_sort.is_empty() || measured {
            self.write_file(id, &record, &stats)?;
        }

        // 7: store the thumbnail of the cover. A source file that no
        // longer opens writes no row, which leaves the book `Unknown`
        // and gives the next `Library::open` the work. A bad cover does
        // not stop an import.
        if let Ok(cover) = source_epub.cover() {
            self.store_cover(id, &Cover::from_file(cover))?;
        }
        Ok(ImportOutcome::Imported { id, made_sort })
    }

    /// Writes a book's `covers` row. `Cover::Unknown` is the missing
    /// row, so it writes nothing.
    fn store_cover(&self, id: i64, cover: &Cover) -> Result<()> {
        let Some(row) = cover.row() else {
            return Ok(());
        };
        self.db.execute(
            "INSERT OR REPLACE INTO covers (book_id, state, image, detail)
             VALUES (?1, ?2, ?3, ?4)",
            params![id, row.state, row.image, row.detail],
        )?;
        Ok(())
    }

    /// Reads the file of each active book that has no `covers` row and
    /// writes the row. `Library::open` calls it, so a book imported
    /// before the covers table existed gets its thumbnail on the first
    /// open after the upgrade, and no read after that opens a zip.
    ///
    /// A book whose file does not open keeps no row, and each open tries
    /// it again for the cost of one failed `File::open`.
    ///
    /// The first open after the upgrade pays for every book at once. One
    /// book costs about a tenth of a second, so a library of 500 books
    /// holds the open for about a minute. Every command opens the
    /// library, so any command can be the one that pays.
    fn fill_covers(&self) -> Result<()> {
        let mut stmt = self
            .db
            .prepare("SELECT id FROM active_books WHERE id NOT IN (SELECT book_id FROM covers)")?;
        let ids: Vec<i64> = stmt
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        drop(stmt);
        for id in ids {
            let Ok(epub) = Epub::open(&self.book_path(id)) else {
                continue;
            };
            let Ok(cover) = epub.cover() else {
                continue;
            };
            self.store_cover(id, &Cover::from_file(cover))?;
        }
        Ok(())
    }

    /// The cover the library holds for a book. A book with no row gives
    /// `Cover::Unknown`. It reads one row, writes nothing, and opens no
    /// file, so a display can call it while it draws. The error is a
    /// database fault.
    pub fn cover(&self, id: i64) -> Result<Cover> {
        let row = self
            .db
            .query_row(
                "SELECT state, image, detail FROM covers WHERE book_id = ?1",
                [id],
                |r| Ok((r.get::<_, String>(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        match row {
            Some((state, image, detail)) => Cover::from_row(&state, image, detail),
            None => Ok(Cover::Unknown),
        }
    }

    fn find_same(&self, record: &Metadata) -> Result<Option<i64>> {
        let Some(first) = record.authors.first() else {
            return Ok(None);
        };
        self.db
            .query_row(
                "SELECT b.id FROM active_books b JOIN book_authors a ON a.book_id = b.id AND a.position = 0
                 WHERE b.title = ?1 AND a.name = ?2 ORDER BY b.id LIMIT 1",
                params![record.title, first.name],
                |row| row.get(0),
            )
            .optional()
            .context("look for the same book")
    }

    fn insert(&mut self, record: &Metadata, stats: &Stats) -> Result<i64> {
        let tx = self.db.transaction()?;
        tx.execute(
            "INSERT INTO books (title, series, series_number, publisher, description) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.title,
                record.series.as_ref().map(|s| &s.name),
                record.series.as_ref().and_then(|s| s.number),
                record.publisher,
                record.description,
            ],
        )?;
        let id = tx.last_insert_rowid();
        insert_authors(&tx, id, &record.authors)?;
        insert_stats(&tx, id, stats)?;
        tx.commit()?;
        Ok(id)
    }

    /// Splices the record and the stats into the book's file.
    fn write_file(&self, id: i64, record: &Metadata, stats: &Stats) -> Result<()> {
        Epub::open(&self.book_path(id))?.write(record, stats)
    }

    /// Every book in id order.
    pub fn list(&self) -> Result<Vec<Book>> {
        let mut stmt = self.db.prepare(&format!("{BOOK_SELECT} ORDER BY id"))?;
        let rows = stmt.query_map([], book_from_row)?;
        let mut books = Vec::new();
        for book in rows {
            let mut book = book?;
            book.metadata.authors = self.authors(book.id)?;
            books.push(book);
        }
        Ok(books)
    }

    pub fn get(&self, id: i64) -> Result<Book> {
        read_book(&self.db, id)
    }

    fn authors(&self, id: i64) -> Result<Vec<Author>> {
        read_authors(&self.db, id)
    }

    /// Updates the row, adds 1 to the revision, then writes the record into
    /// the file, in that order. The stats are written as stored, which is
    /// what the file holds.
    pub fn edit(&mut self, id: i64, record: &Metadata) -> Result<()> {
        let stats = self.get(id)?.stats;
        let tx = self.db.transaction()?;
        tx.execute(
            "UPDATE books SET revision = revision + 1, title = ?2, series = ?3, series_number = ?4,
             publisher = ?5, description = ?6 WHERE id = ?1",
            params![
                id,
                record.title,
                record.series.as_ref().map(|s| &s.name),
                record.series.as_ref().and_then(|s| s.number),
                record.publisher,
                record.description,
            ],
        )?;
        tx.execute("DELETE FROM book_authors WHERE book_id = ?1", [id])?;
        insert_authors(&tx, id, &record.authors)?;
        tx.commit()?;
        self.write_file(id, record, &stats)
    }

    /// Deletes the file, its `sent` rows, and its cover, then sets
    /// `deleted_at` on the book row. The row, its authors, its stats, and
    /// its progress history stay, so the book's word rows and history rows
    /// still have a title.
    pub fn remove(&mut self, id: i64) -> Result<()> {
        self.get(id)?;
        let path = self.book_path(id);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("delete {}", path.display())),
        }
        let tx = self.db.transaction()?;
        tx.execute("DELETE FROM sent WHERE book_id = ?1", [id])?;
        tx.execute("DELETE FROM covers WHERE book_id = ?1", [id])?;
        tx.execute(
            "UPDATE books SET deleted_at = strftime('%Y-%m-%dT%H:%M:%SZ', 'now') WHERE id = ?1",
            [id],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// The schema changes in order. `PRAGMA user_version` counts how many of
/// them the database has, and `Library::open` runs the rest. Each one
/// runs in a transaction, so a change that fails leaves the database as
/// it was.
fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(TABLES_SQL),
        M::up(BOOKS_AUTOINCREMENT_SQL).foreign_key_check(),
        M::up(DELETED_BOOKS_SQL).foreign_key_check(),
        M::up(PROGRESS_HISTORY_SQL).foreign_key_check(),
        M::up(COVERS_SQL),
    ])
}

/// The name of a book's file in the library folder.
pub fn book_file_name(id: i64) -> String {
    format!("{id}.kepub.epub")
}

const BOOK_SELECT: &str =
    "SELECT id, revision, title, series, series_number, publisher, description,
     word_count, reading_ease FROM active_books LEFT JOIN book_stats ON book_id = id";

fn read_book(db: &Connection, id: i64) -> Result<Book> {
    let mut book = db
        .query_row(&format!("{BOOK_SELECT} WHERE id = ?1"), [id], book_from_row)
        .optional()?
        .ok_or_else(|| anyhow!("no book with id {id}"))?;
    book.metadata.authors = read_authors(db, id)?;
    Ok(book)
}

fn read_authors(db: &Connection, id: i64) -> Result<Vec<Author>> {
    let mut stmt =
        db.prepare("SELECT name, sort FROM book_authors WHERE book_id = ?1 ORDER BY position")?;
    let rows = stmt.query_map([id], |row| {
        Ok(Author {
            name: row.get(0)?,
            sort: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A `books` row joined to its `book_stats` row as a Book with no authors.
/// The caller fills them in from `book_authors`.
fn book_from_row(row: &rusqlite::Row) -> rusqlite::Result<Book> {
    let series_name: Option<String> = row.get(3)?;
    let series_number: Option<f64> = row.get(4)?;
    Ok(Book {
        id: row.get(0)?,
        revision: row.get(1)?,
        metadata: Metadata {
            title: row.get(2)?,
            authors: Vec::new(),
            series: series_name.map(|name| Series {
                name,
                number: series_number,
            }),
            publisher: row.get(5)?,
            description: row.get(6)?,
        },
        stats: Stats {
            word_count: row.get::<_, Option<i64>>(7)?.map(|n| n as u64),
            reading_ease: row.get(8)?,
        },
    })
}

/// Inserts the stats row when there is a number to hold.
fn insert_stats(tx: &Transaction, id: i64, stats: &Stats) -> rusqlite::Result<()> {
    if stats.is_empty() {
        return Ok(());
    }
    tx.execute(
        "INSERT INTO book_stats (book_id, word_count, reading_ease) VALUES (?1, ?2, ?3)",
        params![id, stats.word_count.map(|n| n as i64), stats.reading_ease],
    )?;
    Ok(())
}

fn insert_authors(tx: &Transaction, id: i64, authors: &[Author]) -> Result<()> {
    for (position, author) in authors.iter().enumerate() {
        tx.execute(
            "INSERT INTO book_authors (book_id, position, name, sort) VALUES (?1, ?2, ?3, ?4)",
            params![id, position as i64, author.name, author.sort],
        )?;
    }
    Ok(())
}

/// The current reading progress on one device for the book it is keyed
/// by: the newest history row for the book and the device.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ProgressRow {
    pub device_serial: String,
    pub percent: i64,
    pub status: ReadStatus,
    pub last_read: Option<String>,
    /// The Kobo's reading time in seconds.
    pub time_spent: Option<i64>,
    /// When the status last turned finished. Set on the row that first
    /// reads as finished and carried to every row after it, so a book
    /// opened again keeps the date.
    pub finished_at: Option<String>,
}

impl ProgressRow {
    /// The day part of `last_read`: the first ten characters of the
    /// timestamp, as "2026-09-08".
    pub fn day(&self) -> Option<&str> {
        self.last_read.as_deref().map(day_of)
    }

    /// The day part of `finished_at`, as "2026-05-12".
    pub fn finished_day(&self) -> Option<&str> {
        self.finished_at.as_deref().map(day_of)
    }
}

/// The first ten characters of a timestamp, as "2026-09-08".
fn day_of(timestamp: &str) -> &str {
    timestamp.get(..10).unwrap_or(timestamp)
}

/// The columns of the `progress` view after `book_id`, in the order
/// `progress_from_row` reads them.
pub(crate) const PROGRESS_COLUMNS: &str =
    "device_serial, percent, status, last_read, time_spent, finished_at";

/// Reads the `PROGRESS_COLUMNS` that start at column `first`.
pub(crate) fn progress_from_row(r: &rusqlite::Row, first: usize) -> rusqlite::Result<ProgressRow> {
    Ok(ProgressRow {
        device_serial: r.get(first)?,
        percent: r.get(first + 1)?,
        status: r.get(first + 2)?,
        last_read: r.get(first + 3)?,
        time_spent: r.get(first + 4)?,
        finished_at: r.get(first + 5)?,
    })
}

/// One row of a book's progress history: what one sync read for the
/// book on one device. `seen_at` is the time of that sync.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HistoryRow {
    pub device_serial: String,
    pub percent: i64,
    pub status: ReadStatus,
    pub last_read: Option<String>,
    pub time_spent: Option<i64>,
    pub finished_at: Option<String>,
    pub seen_at: String,
}

impl HistoryRow {
    /// The day part of `last_read`, as "2026-09-08".
    pub fn day(&self) -> Option<&str> {
        self.last_read.as_deref().map(day_of)
    }

    /// The day part of `seen_at`, as "2026-09-19".
    pub fn seen_day(&self) -> &str {
        day_of(&self.seen_at)
    }
}

/// One looked-up word, with the title of the book it came from. The
/// title comes from the book row, which stays after the book is removed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WordRow {
    pub word: String,
    pub device_serial: String,
    pub book_id: i64,
    pub book_title: String,
    pub dict_suffix: Option<String>,
    pub looked_up_at: String,
}

impl WordRow {
    /// The day part of `looked_up_at`: the first ten characters of the
    /// timestamp, as "2026-09-08".
    pub fn day(&self) -> &str {
        self.looked_up_at.get(..10).unwrap_or(&self.looked_up_at)
    }
}

impl Library {
    /// Every progress row, grouped by book id and in device order within
    /// a book.
    pub fn progress(&self) -> Result<BTreeMap<i64, Vec<ProgressRow>>> {
        let mut stmt = self.db.prepare(&format!(
            "SELECT book_id, {PROGRESS_COLUMNS} FROM progress ORDER BY book_id, device_serial"
        ))?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, progress_from_row(r, 1)?)))?;
        let mut by_book: BTreeMap<i64, Vec<ProgressRow>> = BTreeMap::new();
        for row in rows {
            let (book_id, progress) = row?;
            by_book.entry(book_id).or_default().push(progress);
        }
        Ok(by_book)
    }

    /// One book's progress rows in device order. A book with no rows, or
    /// no book with that id, gives an empty list.
    pub fn book_progress(&self, book_id: i64) -> Result<Vec<ProgressRow>> {
        let mut stmt = self.db.prepare(&format!(
            "SELECT {PROGRESS_COLUMNS} FROM progress WHERE book_id = ?1 ORDER BY device_serial"
        ))?;
        let rows = stmt.query_map([book_id], |r| progress_from_row(r, 0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// One book's history rows, oldest first. A book with no rows, or no
    /// book with that id, gives an empty list.
    pub fn book_history(&self, book_id: i64) -> Result<Vec<HistoryRow>> {
        let mut stmt = self.db.prepare(
            "SELECT device_serial, percent, status, last_read, time_spent, finished_at, seen_at
             FROM progress_history WHERE book_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([book_id], |r| {
            Ok(HistoryRow {
                device_serial: r.get(0)?,
                percent: r.get(1)?,
                status: r.get(2)?,
                last_read: r.get(3)?,
                time_spent: r.get(4)?,
                finished_at: r.get(5)?,
                seen_at: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The looked-up words, newest first, filtered by book id and device
    /// serial when given.
    pub fn words(&self, book_id: Option<i64>, device_serial: Option<&str>) -> Result<Vec<WordRow>> {
        let mut stmt = self.db.prepare(
            "SELECT w.word, w.device_serial, w.book_id, b.title, w.dict_suffix, w.looked_up_at
             FROM words w JOIN books b ON b.id = w.book_id
             WHERE (?1 IS NULL OR w.book_id = ?1) AND (?2 IS NULL OR w.device_serial = ?2)
             ORDER BY w.looked_up_at DESC, w.id DESC",
        )?;
        let rows = stmt.query_map(params![book_id, device_serial], |r| {
            Ok(WordRow {
                word: r.get(0)?,
                device_serial: r.get(1)?,
                book_id: r.get(2)?,
                book_title: r.get(3)?,
                dict_suffix: r.get(4)?,
                looked_up_at: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_migrations_apply_to_an_empty_database() {
        migrations().validate().unwrap();
    }
}
