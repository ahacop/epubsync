//! Webster's Revised Unabridged Dictionary (1913), for the definition
//! panel in the words pane. The StarDict files in `dictionary/` are
//! embedded in the binary, so a lookup reads no file and needs no network.
//!
//! The `.idx` file lists each headword with the offset and the size of
//! its entry in the unpacked text. It also lists inflected forms, so
//! "Vexed" leads to the entry for "Vex". The first lookup parses the
//! index into a map. The `.dict.dz` file is dictzip: a gzip file whose
//! `RA` header field lists chunks that each unpack on their own, so a
//! lookup unpacks one or two chunks of about 58 KB and not the whole
//! 122 MB.
//!
//! The entries use dictd markup. `plain` removes the markup a reader
//! does not need and keeps the pronunciation codes, such as `[=a]`.

use std::collections::HashMap;
use std::sync::LazyLock;

use anyhow::{Context, bail, ensure};
use flate2::{Decompress, FlushDecompress};

static INDEX_FILE: &[u8] = include_bytes!("../dictionary/web1913.idx");
static DICT_FILE: &[u8] = include_bytes!("../dictionary/web1913.dict.dz");

/// The index: the offset and the size of each entry, by the headword in
/// lower case, in index order. One key can hold more than one entry.
static INDEX: LazyLock<HashMap<String, Vec<(u32, u32)>>> = LazyLock::new(|| index(INDEX_FILE));

/// The entries for `word`, in index order. The lookup ignores case and
/// the white space and punctuation at the two ends of the word. An empty
/// list means that the dictionary has no entry for the word. An error
/// means that the embedded file did not unpack.
pub fn define(word: &str) -> anyhow::Result<Vec<String>> {
    let key = word
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    let Some(records) = INDEX.get(&key) else {
        return Ok(Vec::new());
    };
    let dict = Dictzip::new(DICT_FILE)?;
    records
        .iter()
        .map(|&(offset, size)| {
            let bytes = dict.read(offset as usize, size as usize)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        })
        .collect()
}

/// Reads the index: per record, a word that ends in a NUL byte, then the
/// offset and the size as big-endian `u32` values. A record cut short at
/// the end of the file is dropped.
fn index(bytes: &[u8]) -> HashMap<String, Vec<(u32, u32)>> {
    let mut map: HashMap<String, Vec<(u32, u32)>> = HashMap::new();
    let mut rest = bytes;
    while let Some(nul) = rest.iter().position(|&b| b == 0) {
        let Some(numbers) = rest.get(nul + 1..nul + 9) else {
            break;
        };
        let word = String::from_utf8_lossy(&rest[..nul]).to_lowercase();
        let offset = u32::from_be_bytes(numbers[..4].try_into().unwrap());
        let size = u32::from_be_bytes(numbers[4..].try_into().unwrap());
        map.entry(word).or_default().push((offset, size));
        rest = &rest[nul + 9..];
    }
    map
}

/// An entry as the panel shows it. At the start of a line, the headword
/// before ` \` goes, since the spelled headword that follows it repeats
/// it. The backslashes around the spelled headword and the braces around
/// a cross-reference go too. The white space at the end goes.
///
/// `Hale \Hale\ (h[=a]l), a. See {Whole}.` becomes
/// `Hale (h[=a]l), a. See Whole.`
pub fn plain(entry: &str) -> String {
    entry
        .trim_end()
        .lines()
        .map(|line| {
            let starts_entry = !line.starts_with(char::is_whitespace);
            let line = match line.find(" \\") {
                Some(i) if starts_entry => &line[i + 1..],
                _ => line,
            };
            line.replace(['\\', '{', '}'], "")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A dictzip file over a byte slice: the chunk length, and the start and
/// the packed length of each chunk.
struct Dictzip<'a> {
    bytes: &'a [u8],
    chunk_len: usize,
    chunks: Vec<(usize, usize)>,
}

const FHCRC: u8 = 0x02;
const FEXTRA: u8 = 0x04;
const FNAME: u8 = 0x08;
const FCOMMENT: u8 = 0x10;

impl<'a> Dictzip<'a> {
    /// Reads the gzip header. The `RA` subfield of the extra field gives
    /// the chunk length and the packed size of each chunk. The name, the
    /// comment, and the header CRC are skipped when the flags set them.
    fn new(bytes: &'a [u8]) -> anyhow::Result<Self> {
        ensure!(
            bytes.len() >= 12 && bytes[..3] == [0x1f, 0x8b, 8],
            "not a gzip file"
        );
        let flags = bytes[3];
        ensure!(flags & FEXTRA != 0, "the gzip header has no extra field");
        let extra_len = u16::from_le_bytes([bytes[10], bytes[11]]) as usize;
        let mut pos = 12 + extra_len;
        let extra = bytes.get(12..pos).context("the extra field is cut short")?;
        let (chunk_len, sizes) = random_access(extra)?;
        for flag in [FNAME, FCOMMENT] {
            if flags & flag != 0 {
                let nul = bytes[pos..]
                    .iter()
                    .position(|&b| b == 0)
                    .context("the gzip header is cut short")?;
                pos += nul + 1;
            }
        }
        if flags & FHCRC != 0 {
            pos += 2;
        }
        let mut chunks = Vec::with_capacity(sizes.len());
        for size in sizes {
            chunks.push((pos, size));
            pos += size;
        }
        ensure!(
            pos <= bytes.len(),
            "the chunks run past the end of the file"
        );
        Ok(Dictzip {
            bytes,
            chunk_len,
            chunks,
        })
    }

    /// The `size` bytes at `offset` in the unpacked text. Each chunk that
    /// holds a part of the range unpacks with raw inflate.
    fn read(&self, offset: usize, size: usize) -> anyhow::Result<Vec<u8>> {
        if size == 0 {
            return Ok(Vec::new());
        }
        let first = offset / self.chunk_len;
        let last = (offset + size - 1) / self.chunk_len;
        ensure!(
            last < self.chunks.len(),
            "the range runs past the last chunk"
        );
        let mut text = Vec::with_capacity((last - first + 1) * self.chunk_len);
        for &(start, len) in &self.chunks[first..=last] {
            let mut out = Vec::with_capacity(self.chunk_len);
            Decompress::new(false)
                .decompress_vec(
                    &self.bytes[start..start + len],
                    &mut out,
                    FlushDecompress::Sync,
                )
                .context("a chunk did not unpack")?;
            text.extend_from_slice(&out);
        }
        let skip = offset - first * self.chunk_len;
        text.get(skip..skip + size)
            .map(<[u8]>::to_vec)
            .context("a chunk unpacked short")
    }
}

/// Reads the `RA` subfield out of a gzip extra field: the chunk length
/// and the packed size of each chunk. The field is a list of subfields,
/// each two id bytes, a little-endian length, and the data.
fn random_access(mut extra: &[u8]) -> anyhow::Result<(usize, Vec<usize>)> {
    while extra.len() >= 4 {
        let len = u16::from_le_bytes([extra[2], extra[3]]) as usize;
        let data = extra.get(4..4 + len).context("a subfield is cut short")?;
        if extra[..2] == *b"RA" {
            let words: Vec<usize> = data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&w| u16::from_le_bytes(w) as usize)
                .collect();
            let [_version, chunk_len, count, sizes @ ..] = words.as_slice() else {
                bail!("the RA subfield is cut short");
            };
            ensure!(*chunk_len > 0, "the chunk length is 0");
            ensure!(
                sizes.len() == *count,
                "the RA subfield lists {} of {count} chunks",
                sizes.len()
            );
            return Ok((*chunk_len, sizes.to_vec()));
        }
        extra = &extra[4 + len..];
    }
    bail!("the gzip header has no RA subfield")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::{Compress, Compression, FlushCompress};

    /// A dictzip file of `text` in chunks of `chunk_len` bytes, with a
    /// file name in the header. Each chunk is raw deflate that ends in a
    /// full flush, so it unpacks on its own.
    fn dictzip(text: &[u8], chunk_len: usize) -> Vec<u8> {
        let mut packed = Vec::new();
        let mut sizes = Vec::new();
        for chunk in text.chunks(chunk_len) {
            let mut out = Vec::with_capacity(chunk.len() * 2 + 64);
            Compress::new(Compression::default(), false)
                .compress_vec(chunk, &mut out, FlushCompress::Full)
                .unwrap();
            sizes.push(out.len() as u16);
            packed.extend(out);
        }
        let mut ra = vec![1u16, chunk_len as u16, sizes.len() as u16];
        ra.extend(&sizes);
        let ra: Vec<u8> = ra.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut extra = b"RA".to_vec();
        extra.extend((ra.len() as u16).to_le_bytes());
        extra.extend(ra);

        let mut file = vec![0x1f, 0x8b, 8, FEXTRA | FNAME, 0, 0, 0, 0, 2, 3];
        file.extend((extra.len() as u16).to_le_bytes());
        file.extend(extra);
        file.extend(b"test.dict\0");
        file.extend(packed);
        file
    }

    #[test]
    fn a_read_inside_one_chunk() {
        let text: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let file = dictzip(&text, 100);
        let dict = Dictzip::new(&file).unwrap();
        assert_eq!(dict.chunks.len(), 10);
        assert_eq!(dict.read(210, 50).unwrap(), &text[210..260]);
        assert_eq!(dict.read(0, 100).unwrap(), &text[..100]);
    }

    #[test]
    fn a_read_across_chunk_boundaries() {
        let text: Vec<u8> = (0..1000u32).map(|i| (i % 251) as u8).collect();
        let file = dictzip(&text, 100);
        let dict = Dictzip::new(&file).unwrap();
        assert_eq!(dict.read(150, 300).unwrap(), &text[150..450]);
        assert_eq!(dict.read(990, 10).unwrap(), &text[990..]);
    }

    #[test]
    fn a_read_past_the_end_fails() {
        let file = dictzip(&[b'a'; 250], 100);
        let dict = Dictzip::new(&file).unwrap();
        assert!(dict.read(200, 100).is_err());
    }

    #[test]
    fn a_file_with_no_ra_subfield_fails() {
        let file = [
            0x1f, 0x8b, 8, FEXTRA, 0, 0, 0, 0, 2, 3, 4, 0, b'X', b'Y', 0, 0,
        ];
        assert!(Dictzip::new(&file).is_err());
    }

    #[test]
    fn the_embedded_file_has_its_chunks() {
        let dict = Dictzip::new(DICT_FILE).unwrap();
        assert_eq!(dict.chunk_len, 58315);
        assert_eq!(dict.chunks.len(), 2088);
    }

    #[test]
    fn the_index_keys_are_lower_case_and_keep_their_order() {
        let mut bytes = Vec::new();
        for (word, offset, size) in [("Hale", 10u32, 5u32), ("hale", 20, 6), ("Vex", 30, 7)] {
            bytes.extend(word.as_bytes());
            bytes.push(0);
            bytes.extend(offset.to_be_bytes());
            bytes.extend(size.to_be_bytes());
        }
        let map = index(&bytes);
        assert_eq!(map["hale"], [(10, 5), (20, 6)]);
        assert_eq!(map["vex"], [(30, 7)]);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn the_embedded_index_has_every_record() {
        let count: usize = INDEX.values().map(Vec::len).sum();
        assert_eq!(count, 160_161);
    }

    #[test]
    fn the_largest_entry_reads_across_chunks() {
        let dict = Dictzip::new(DICT_FILE).unwrap();
        let &(offset, size) = INDEX.values().flatten().max_by_key(|r| r.1).unwrap();
        assert_eq!(size, 75_851);
        let bytes = dict.read(offset as usize, size as usize).unwrap();
        assert_eq!(bytes.len(), 75_851);
        assert!(std::str::from_utf8(&bytes).is_ok());
    }

    #[test]
    fn vex_has_its_entry() {
        let entries = define("vex").unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].starts_with("Vex \\Vex\\, v. t."));
        assert!(entries[0].contains("To make angry"));
    }

    #[test]
    fn an_inflected_form_leads_to_the_entry() {
        let entries = define("Vexed").unwrap();
        assert!(entries[0].contains("To make angry"));
    }

    #[test]
    fn the_lookup_ignores_case_and_punctuation_at_the_ends() {
        assert_eq!(define("VEX").unwrap(), define("vex").unwrap());
        assert_eq!(define(" “vex,” ").unwrap(), define("vex").unwrap());
    }

    #[test]
    fn a_word_with_no_entry_gives_an_empty_list() {
        assert!(define("zzzq").unwrap().is_empty());
        assert!(define("").unwrap().is_empty());
    }

    #[test]
    fn plain_drops_the_headword_before_the_spelled_headword() {
        assert_eq!(
            plain("Vex \\Vex\\, v. t.\n   To agitate."),
            "Vex, v. t.\n   To agitate."
        );
        assert_eq!(
            plain("Abundant number \\A*bun\"dant num\"ber\\ (Math.)"),
            "A*bun\"dant num\"ber (Math.)"
        );
    }

    #[test]
    fn plain_keeps_an_indented_line_whole() {
        assert_eq!(
            plain("Run \\Run\\, v.\n   a line \\with\\ a mark"),
            "Run, v.\n   a line with a mark"
        );
    }

    #[test]
    fn plain_drops_the_braces_and_keeps_the_pronunciation() {
        assert_eq!(
            plain("Hale \\Hale\\ (h[=a]l), a. See {Whole}.\n\n"),
            "Hale (h[=a]l), a. See Whole."
        );
    }

    #[test]
    fn plain_handles_each_headword_line_of_hale() {
        let entry = plain(&define("hale").unwrap()[0]);
        let heads: Vec<&str> = entry.lines().filter(|l| l.starts_with("Hale")).collect();
        assert_eq!(
            heads,
            [
                "Hale (h[=a]l), a. [Written also hail.] [OE. heil, Icel.",
                "Hale, n.",
                "Hale (h[=a]l or h[add]l; 277), v. t. [imp. & p. p.",
            ]
        );
        assert!(!entry.contains(['\\', '{', '}']));
    }
}
