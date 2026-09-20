//! Reads one entry out of an EPUB zip, and rebuilds the zip with one
//! entry replaced. A rebuild copies every other entry raw, so its
//! compressed bytes, its order, and its method stay.

#[cfg(test)]
mod tests;

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};
use zip::CompressionMethod;
use zip::write::SimpleFileOptions;

/// Writes `new_opf` in place of the entry at `opf_path` inside the EPUB at
/// `path`. The new zip is built in a temp file next to the original and
/// renamed over it, so the original is whole until the new zip is
/// complete, and a rewrite that fails removes the temp file when it
/// returns. `mimetype` stays first and stored because it is copied raw
/// in its original position.
pub fn rewrite(path: &Path, opf_path: &str, new_opf: &str) -> Result<()> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("read {}", path.display()))?;

    let folder = path.parent().unwrap_or(Path::new("."));
    let temp = tempfile::Builder::new()
        .prefix(".epubsync-")
        .suffix(".tmp")
        .tempfile_in(folder)
        .with_context(|| format!("create a temp file in {}", folder.display()))?;
    let mut writer = zip::ZipWriter::new(BufWriter::new(temp));

    let mut wrote_opf = false;
    for i in 0..archive.len() {
        let entry = archive
            .by_index_raw(i)
            .with_context(|| format!("read entry {i}"))?;
        if entry.name() == opf_path {
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .last_modified_time(entry.last_modified().unwrap_or_default());
            writer.start_file(opf_path, options)?;
            writer.write_all(new_opf.as_bytes())?;
            wrote_opf = true;
        } else {
            writer.raw_copy_file(entry)?;
        }
    }
    anyhow::ensure!(wrote_opf, "no {opf_path} in {}", path.display());

    let mut out = writer.finish()?;
    out.flush()?;
    out.get_ref().as_file().sync_all()?;
    let temp = out.into_inner().map_err(|e| e.into_error())?;
    temp.persist(path)
        .map_err(|e| e.error)
        .with_context(|| format!("replace {}", path.display()))?;
    Ok(())
}

/// What a zip gives for one entry name.
pub enum Entry {
    /// The bytes of the entry.
    Bytes(Vec<u8>),
    /// The zip opened and does not give the entry: it holds no such
    /// name, or the entry did not read. The text says which.
    Missing(String),
}

/// Reads the entry named `name` out of the EPUB at `path`. The error is
/// the file itself: it did not open, or it is not a zip. A fault in the
/// one entry is `Entry::Missing`, because the rest of the file is still
/// good and the caller can go on without it.
pub fn read_entry(path: &Path, name: &str) -> Result<Entry> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut archive = zip::ZipArchive::new(BufReader::new(file))
        .with_context(|| format!("read {}", path.display()))?;
    let mut entry = match archive.by_name(name) {
        Ok(entry) => entry,
        Err(e) => return Ok(Entry::Missing(format!("{name}: {e}"))),
    };
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    match std::io::copy(&mut entry, &mut bytes) {
        Ok(_) => Ok(Entry::Bytes(bytes)),
        Err(e) => Ok(Entry::Missing(format!("{name}: {e}"))),
    }
}
