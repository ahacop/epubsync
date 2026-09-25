//! A Kobo over USB mass storage: a mounted volume with a `.kobo/version`
//! file. Books go into the `EpubSync` folder at the root of the volume as
//! `<id>.kepub.epub`. The Kobo database is opened in place for the row
//! updates and the read back.

pub mod db;
pub mod eject;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};

use crate::device::{Action, Device, ReadBack, RowUpdate};
use crate::metadata::Metadata;
use db::KoboDb;

pub const FOLDER: &str = "EpubSync";
const VERSION_FILE: &str = ".kobo/version";

pub struct Kobo {
    pub serial: String,
    pub root: PathBuf,
    db: Db,
}

/// The Kobo database as this session may use it.
enum Db {
    /// The volume has no `.kobo/KoboReader.sqlite`. Also the state before
    /// `open_db` runs and after `finish` closes the database.
    Missing,
    Writable(KoboDb),
    /// An untested `dbversion`. Reads run. Row writes are refused.
    ReadOnly {
        db: KoboDb,
        reason: String,
    },
}

/// The mount folders detect scans: `/run/media/$USER`, `/media`,
/// `/media/$USER`, and `/Volumes`.
pub fn default_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/media"), PathBuf::from("/Volumes")];
    if let Ok(user) = std::env::var("USER") {
        roots.insert(0, PathBuf::from("/run/media").join(&user));
        roots.insert(2, PathBuf::from("/media").join(&user));
    }
    roots
}

/// Finds every mounted volume directly under one of `roots` that has a
/// `.kobo/version` file.
pub fn detect(roots: &[PathBuf]) -> Vec<Kobo> {
    let mut found = Vec::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            if let Ok(kobo) = Kobo::at(&path) {
                found.push(kobo);
            }
        }
    }
    found
}

/// The one Kobo under `roots`. No Kobo and more than one Kobo are both
/// errors, because a sync or an eject is for one device. The error for
/// more than one names each root.
pub fn detect_one(roots: &[PathBuf]) -> Result<Kobo> {
    let mut found = detect(roots);
    match found.len() {
        0 => bail!("no Kobo found"),
        1 => Ok(found.remove(0)),
        _ => {
            let paths: Vec<String> = found.iter().map(|k| k.root.display().to_string()).collect();
            bail!("more than one Kobo found: {}", paths.join(", "))
        }
    }
}

impl Kobo {
    /// Opens the volume at `root`, which must hold `.kobo/version`. The
    /// first field of that file is the serial.
    pub fn at(root: &Path) -> Result<Kobo> {
        let version_path = root.join(VERSION_FILE);
        let text = std::fs::read_to_string(&version_path)
            .with_context(|| format!("{} is not a Kobo: no {VERSION_FILE}", root.display()))?;
        let serial = text.split(',').next().unwrap_or("").trim().to_string();
        if serial.is_empty() {
            return Err(anyhow!("{} has an empty serial", version_path.display()));
        }
        Ok(Kobo {
            serial,
            root: root.to_path_buf(),
            db: Db::Missing,
        })
    }

    /// Opens `.kobo/KoboReader.sqlite` when the volume has one. With
    /// `allow_untested` the row writes run on any database version.
    pub fn open_db(&mut self, allow_untested: bool) -> Result<()> {
        let path = self.root.join(db::DB_PATH);
        if !path.exists() {
            self.db = Db::Missing;
            return Ok(());
        }
        let db = KoboDb::open(&path)?;
        self.db = if db.is_tested() || allow_untested {
            Db::Writable(db)
        } else {
            let reason = format!(
                "Kobo database version {} has not been tested. Replacements and row updates are skipped. \
                 Pass --allow-newer-firmware to run them.",
                db.version
            );
            Db::ReadOnly { db, reason }
        };
        Ok(())
    }

    pub fn db_version(&self) -> Option<i64> {
        match &self.db {
            Db::Missing => None,
            Db::Writable(db) | Db::ReadOnly { db, .. } => Some(db.version),
        }
    }

    pub fn folder(&self) -> PathBuf {
        self.root.join(FOLDER)
    }

    pub fn book_path(&self, id: i64) -> PathBuf {
        self.folder().join(format!("{id}.kepub.epub"))
    }

    /// The path the firmware keys the book's rows by.
    pub fn volume_id(&self, id: i64) -> String {
        format!("file:///mnt/onboard/{FOLDER}/{id}.kepub.epub")
    }

    /// Deletes the `._` files in the book folder and returns how many it
    /// deleted. macOS writes a `._` file to hold a copied file's extended
    /// attributes on a FAT volume.
    pub fn remove_dot_underscore_files(&self) -> Result<usize> {
        let entries = match std::fs::read_dir(self.folder()) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e).with_context(|| format!("read {}", self.folder().display())),
        };
        let mut removed = 0;
        for entry in entries {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with("._") && entry.file_type()?.is_file()
            {
                std::fs::remove_file(entry.path())
                    .with_context(|| format!("delete {}", entry.path().display()))?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Unmounts the volume and tells the Kobo the session is over. Call
    /// after `finish`, so the database is closed first.
    pub fn eject(&self) -> Result<eject::Ejected> {
        eject::eject(&self.root)
    }
}

/// Copies the file's bytes to `target` and flushes them to the volume.
/// Returns the size.
///
/// `std::fs::copy` is not used because on macOS it also copies extended
/// attributes. FAT cannot store them, so macOS writes them to a
/// `._<name>` file next to the book.
fn copy_bytes(source: &Path, target: &Path) -> Result<u64> {
    let mut from = std::fs::File::open(source)?;
    let mut to = std::fs::File::create(target)?;
    let size = std::io::copy(&mut from, &mut to)?;
    to.sync_all()?;
    Ok(size)
}

/// Parses the book id out of a device file name such as `12.kepub.epub`.
pub fn id_from_file_name(name: &str) -> Option<i64> {
    name.strip_suffix(".kepub.epub")?.parse().ok()
}

/// Parses the book id out of a volume id such as
/// `file:///mnt/onboard/EpubSync/12.kepub.epub`.
pub fn id_from_volume_id(volume_id: &str) -> Option<i64> {
    let name = volume_id.strip_prefix(&format!("file:///mnt/onboard/{FOLDER}/"))?;
    id_from_file_name(name)
}

impl Device for Kobo {
    fn serial(&self) -> &str {
        &self.serial
    }

    fn list(&self) -> Result<BTreeSet<i64>> {
        let entries = match std::fs::read_dir(self.folder()) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(e) => return Err(e).with_context(|| format!("read {}", self.folder().display())),
        };
        Ok(entries
            .filter_map(|e| e.ok())
            .filter_map(|e| id_from_file_name(&e.file_name().to_string_lossy()))
            .collect())
    }

    fn apply(&mut self, action: &Action, source: &Path) -> Result<()> {
        let id = action.id();
        let target = self.book_path(id);
        let volume_id = self.volume_id(id);
        match action {
            Action::Send { .. } | Action::Replace { .. } | Action::SendAgain { .. } => {
                std::fs::create_dir_all(self.folder())?;
                if !matches!(action, Action::Replace { .. })
                    && let Db::Writable(db) = &self.db
                {
                    db.delete_stale_content(&volume_id)?;
                }
                let size = copy_bytes(source, &target)
                    .with_context(|| format!("copy to {}", target.display()))?;
                if matches!(action, Action::Replace { .. })
                    && let Db::Writable(db) = &self.db
                {
                    db.update_file_size(&volume_id, size as i64)?;
                }
            }
            Action::Delete { .. } => {
                std::fs::remove_file(&target)
                    .with_context(|| format!("delete {}", target.display()))?;
            }
        }
        Ok(())
    }

    fn write_gate(&self) -> Option<String> {
        match &self.db {
            Db::ReadOnly { reason, .. } => Some(reason.clone()),
            Db::Missing | Db::Writable(_) => None,
        }
    }

    fn update_rows(&mut self, records: &[(i64, &Metadata)]) -> Result<Vec<(i64, RowUpdate)>> {
        let db = match &mut self.db {
            Db::Missing => {
                return Ok(records
                    .iter()
                    .map(|(id, _)| (*id, RowUpdate::NoRow))
                    .collect());
            }
            Db::ReadOnly { reason, .. } => bail!("{reason}"),
            Db::Writable(db) => db,
        };
        let tx = db.begin()?;
        let mut results = Vec::new();
        for (id, record) in records {
            let volume_id = format!("file:///mnt/onboard/{FOLDER}/{id}.kepub.epub");
            results.push((*id, db::update_metadata(&tx, &volume_id, record)?));
        }
        tx.commit()?;
        Ok(results)
    }

    fn read_back(&mut self, book_ids: &[i64]) -> Result<ReadBack> {
        let db = match &self.db {
            Db::Missing => return Ok(ReadBack::default()),
            Db::Writable(db) | Db::ReadOnly { db, .. } => db,
        };
        let mut progress = Vec::new();
        for id in book_ids {
            if let Some(p) = db.progress(&self.volume_id(*id), *id)? {
                progress.push(p);
            }
        }
        let words =
            db.words(|volume_id| id_from_volume_id(volume_id).filter(|id| book_ids.contains(id)))?;
        Ok(ReadBack {
            progress,
            words,
            changed: 0,
        })
    }

    fn finish(&mut self) -> Result<()> {
        match std::mem::replace(&mut self.db, Db::Missing) {
            Db::Missing => Ok(()),
            Db::Writable(db) | Db::ReadOnly { db, .. } => db.close(),
        }
    }
}
