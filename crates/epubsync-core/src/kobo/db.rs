//! The Kobo database, `.kobo/KoboReader.sqlite`, opened in place while the
//! volume is mounted. Only sync touches it.
//!
//! The column names and the file size rule are not documented by Kobo.
//! They come from Calibre's Kobo driver. Every write is gated on the
//! `dbversion` value: on a version not in `TESTED_VERSIONS` the caller
//! refuses replacements and row updates unless told otherwise.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::device::{Progress, ReadStatus, RowUpdate, Word};
use crate::metadata::{Metadata, format_series_number};

/// The `dbversion` values the app has been run against on a real Kobo.
/// Add a value after the device checks in the implementation plan pass.
pub const TESTED_VERSIONS: &[i64] = &[174];

pub const DB_PATH: &str = ".kobo/KoboReader.sqlite";

pub struct KoboDb {
    conn: Connection,
    pub version: i64,
}

/// A book's `content` row, the columns the app keeps equal to the record.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentRow {
    pub title: Option<String>,
    pub attribution: Option<String>,
    pub series: Option<String>,
    pub series_number: Option<String>,
    pub series_id: Option<String>,
    pub series_number_float: Option<f64>,
    pub description: Option<String>,
    pub publisher: Option<String>,
    pub file_size: i64,
}

impl KoboDb {
    /// Opens the database file and reads its version.
    pub fn open(path: &Path) -> Result<KoboDb> {
        let conn = Connection::open(path).with_context(|| format!("open {}", path.display()))?;
        let version: i64 = conn
            .query_row("SELECT version FROM dbversion", [], |r| r.get(0))
            .context("read dbversion")?;
        Ok(KoboDb { conn, version })
    }

    pub fn is_tested(&self) -> bool {
        TESTED_VERSIONS.contains(&self.version)
    }

    /// The `content` row for a book path, when the firmware has made one.
    pub fn find_content(&self, volume_id: &str) -> Result<Option<ContentRow>> {
        find_content(&self.conn, volume_id)
    }

    /// Compares the row's title, attribution, series columns, description,
    /// and publisher to the record and writes the ones that differ.
    pub fn update_metadata(&self, volume_id: &str, record: &Metadata) -> Result<RowUpdate> {
        update_metadata(&self.conn, volume_id, record)
    }

    /// Starts a transaction for a run of `update_metadata` calls.
    pub fn begin(&mut self) -> Result<Transaction<'_>> {
        Ok(self.conn.transaction()?)
    }

    /// Sets the stored file size after a replacement, so the firmware does
    /// not treat the file as a new book. Returns false when there is no row.
    pub fn update_file_size(&self, volume_id: &str, size: i64) -> Result<bool> {
        let n = self.conn.execute(
            "UPDATE content SET ___FileSize = ?2 WHERE ContentID = ?1 AND ContentType = '6'",
            params![volume_id, size],
        )?;
        Ok(n > 0)
    }

    /// Deletes the `content` row the firmware can leave behind when a book
    /// is deleted on the device, so a copy of the book to the same path
    /// gets a fresh row at the next scan. A book copied on top of that row
    /// can stay hidden. The row has `Accessibility` 1 and `IsDownloaded`
    /// false, the form Calibre's driver deletes. A live row is left alone.
    /// Returns false when there is no such row.
    pub fn delete_stale_content(&self, volume_id: &str) -> Result<bool> {
        let n = self.conn.execute(
            "DELETE FROM content WHERE ContentID = ?1 AND ContentType = '6'
             AND Accessibility = 1 AND IsDownloaded IN ('false', 0)",
            [volume_id],
        )?;
        Ok(n > 0)
    }

    /// The percent read, the read status, the last read time, and the
    /// reading time for a book path. `book_id` is copied into the result.
    pub fn progress(&self, volume_id: &str, book_id: i64) -> Result<Option<Progress>> {
        self.conn
            .query_row(
                "SELECT ___PercentRead, ReadStatus, DateLastRead, TimeSpentReading
                 FROM content WHERE ContentID = ?1 AND ContentType = '6'",
                [volume_id],
                |r| {
                    Ok(Progress {
                        book_id,
                        percent: r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                        status: r
                            .get::<_, Option<ReadStatus>>(1)?
                            .unwrap_or(ReadStatus::Unread),
                        last_read: r.get(2)?,
                        time_spent: r.get(3)?,
                    })
                },
            )
            .optional()
            .context("read progress")
    }

    /// The `WordList` rows from library books. `book_id_for` maps a
    /// volume id to a library book id, and a row whose volume id maps to
    /// no book is skipped.
    pub fn words(&self, book_id_for: impl Fn(&str) -> Option<i64>) -> Result<Vec<Word>> {
        let mut stmt = self.conn.prepare(
            "SELECT Text, VolumeId, DictSuffix, DateCreated FROM WordList ORDER BY DateCreated",
        )?;
        let rows = stmt.query_map([], |r| {
            let volume_id: String = r.get(1)?;
            let Some(book_id) = book_id_for(&volume_id) else {
                return Ok(None);
            };
            Ok(Some(Word {
                word: r.get(0)?,
                book_id,
                dict_suffix: r.get(2)?,
                looked_up_at: r.get::<_, Option<String>>(3)?.unwrap_or_default(),
            }))
        })?;
        let mut words = Vec::new();
        for row in rows {
            words.extend(row?);
        }
        Ok(words)
    }

    pub fn close(self) -> Result<()> {
        self.conn
            .close()
            .map_err(|(_, e)| e)
            .context("close the Kobo database")
    }
}

fn find_content(conn: &Connection, volume_id: &str) -> Result<Option<ContentRow>> {
    conn.query_row(
        "SELECT Title, Attribution, Series, SeriesNumber, SeriesID, SeriesNumberFloat, Description, Publisher, ___FileSize
         FROM content WHERE ContentID = ?1 AND ContentType = '6'",
        [volume_id],
        |r| {
            Ok(ContentRow {
                title: r.get(0)?,
                attribution: r.get(1)?,
                series: r.get(2)?,
                series_number: r.get(3)?,
                series_id: r.get(4)?,
                series_number_float: r.get(5)?,
                description: r.get(6)?,
                publisher: r.get(7)?,
                file_size: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
            })
        },
    )
    .optional()
    .context("read the content row")
}

/// Works on a connection or a transaction, which derefs to one.
pub fn update_metadata(conn: &Connection, volume_id: &str, record: &Metadata) -> Result<RowUpdate> {
    let Some(row) = find_content(conn, volume_id)? else {
        return Ok(RowUpdate::NoRow);
    };
    let wanted = ContentRow {
        title: Some(record.title.clone()),
        attribution: Some(attribution(record)),
        series: record.series.as_ref().map(|s| s.name.clone()),
        series_number: record
            .series
            .as_ref()
            .and_then(|s| s.number)
            .map(format_series_number),
        series_id: record.series.as_ref().map(|s| s.name.clone()),
        series_number_float: record.series.as_ref().and_then(|s| s.number),
        description: record.description.clone(),
        publisher: record.publisher.clone(),
        file_size: row.file_size,
    };
    if wanted == row {
        return Ok(RowUpdate::Unchanged);
    }
    conn.execute(
        "UPDATE content SET Title = ?2, Attribution = ?3, Series = ?4, SeriesNumber = ?5, SeriesID = ?6,
         SeriesNumberFloat = ?7, Description = ?8, Publisher = ?9
         WHERE ContentID = ?1 AND ContentType = '6'",
        params![
            volume_id,
            wanted.title,
            wanted.attribution,
            wanted.series,
            wanted.series_number,
            wanted.series_id,
            wanted.series_number_float,
            wanted.description,
            wanted.publisher,
        ],
    )?;
    Ok(RowUpdate::Updated)
}

/// The record's authors joined with " & ", the string the Kobo shows.
pub fn attribution(record: &Metadata) -> String {
    record
        .authors
        .iter()
        .map(|a| a.name.as_str())
        .collect::<Vec<_>>()
        .join(" & ")
}
