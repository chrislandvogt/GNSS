//! Projects as zip files (ARCHITECTURE.md section 14.2): how a project moves
//! in and out of browser storage, and how one is handed to someone else.
//!
//! Only what a project needs: files stored or deflated, with UTF-8 names,
//! no encryption and no ZIP64. Exports are deterministic: files in path order,
//! a fixed timestamp, and the same compression on every platform, so the same
//! project always gives the same bytes.

use miniz_oxide::deflate::compress_to_vec;
use miniz_oxide::inflate::decompress_to_vec_with_limit;

use crate::{MemoryStore, ProjectPath, Store};

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const END: u32 = 0x0605_4b50;
/// 1980-01-01 00:00, the earliest time a zip can hold: exports carry no clock.
const DOS_DATE: u16 = (1 << 5) | 1;
/// Names are UTF-8.
const UTF8: u16 = 1 << 11;
/// The most one file may expand to, and the most a whole zip may: well
/// beyond any RINEX file or project, and a stop for zip bombs.
const MAX_FILE: usize = 2 << 30;
const MAX_TOTAL: usize = 8 << 30;

/// Why a zip couldn't be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ZipError {
    /// It isn't a zip, or it's damaged.
    #[error("this isn't a readable zip file: {0}")]
    Malformed(String),
    /// It uses something projects don't need.
    #[error("{0}; save the zip again with standard settings (deflate, no encryption)")]
    Unsupported(String),
    /// A name that would land outside the project.
    #[error("{0:?} isn't a path inside a project, so it's refused")]
    BadName(String),
    /// A file whose contents don't match its checksum.
    #[error("{0} is damaged (its CRC doesn't match)")]
    Damaged(String),
    /// Too big to be a project, or a file that expands absurdly.
    #[error("{0} is too large to be part of a project")]
    TooLarge(String),
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(bytes);
    hasher.finalize()
}

fn u16_le(v: u16) -> [u8; 2] {
    v.to_le_bytes()
}
fn u32_le(v: u32) -> [u8; 4] {
    v.to_le_bytes()
}

/// A project as a zip, every file in path order except `.cache/`, which can
/// be made again.
pub fn export(store: &MemoryStore) -> Result<Vec<u8>, ZipError> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut count: u16 = 0;
    for (path, bytes) in store.files() {
        if path.as_str().starts_with(".cache/") {
            continue;
        }
        let name = path.as_str().as_bytes();
        let too_large = || ZipError::TooLarge(path.to_string());
        let size = u32::try_from(bytes.len()).map_err(|_| too_large())?;
        let deflated = compress_to_vec(bytes, 6);
        let (method, data): (u16, &[u8]) = if deflated.len() < bytes.len() {
            (8, &deflated)
        } else {
            (0, bytes)
        };
        let compressed = u32::try_from(data.len()).map_err(|_| too_large())?;
        let offset = u32::try_from(out.len()).map_err(|_| too_large())?;
        let crc = crc32(bytes);
        let name_len = u16::try_from(name.len()).map_err(|_| too_large())?;
        // Version 2.0, UTF-8 names, method, fixed time, CRC, sizes, name.
        let common = [
            &u16_le(20)[..],
            &u16_le(UTF8),
            &u16_le(method),
            &u16_le(0),
            &u16_le(DOS_DATE),
            &u32_le(crc),
            &u32_le(compressed),
            &u32_le(size),
            &u16_le(name_len),
            &u16_le(0),
        ]
        .concat();
        out.extend_from_slice(&u32_le(LOCAL));
        out.extend_from_slice(&common);
        out.extend_from_slice(name);
        out.extend_from_slice(data);

        central.extend_from_slice(&u32_le(CENTRAL));
        central.extend_from_slice(&u16_le(20)); // made by: 2.0, MS-DOS attributes
        central.extend_from_slice(&common);
        central.extend_from_slice(&u16_le(0)); // comment length
        central.extend_from_slice(&u16_le(0)); // disk
        central.extend_from_slice(&u16_le(0)); // internal attributes
        central.extend_from_slice(&u32_le(0)); // external attributes
        central.extend_from_slice(&u32_le(offset));
        central.extend_from_slice(name);
        count = count
            .checked_add(1)
            .ok_or_else(|| ZipError::TooLarge("the project (over 65,535 files)".into()))?;
    }
    let directory_offset =
        u32::try_from(out.len()).map_err(|_| ZipError::TooLarge("the project".into()))?;
    let directory_size =
        u32::try_from(central.len()).map_err(|_| ZipError::TooLarge("the project".into()))?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&u32_le(END));
    out.extend_from_slice(&[0; 4]); // this disk, the directory's disk
    out.extend_from_slice(&u16_le(count));
    out.extend_from_slice(&u16_le(count));
    out.extend_from_slice(&u32_le(directory_size));
    out.extend_from_slice(&u32_le(directory_offset));
    out.extend_from_slice(&u16_le(0)); // comment length
    Ok(out)
}

/// Little-endian reads that fail rather than panic past the end.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn bytes(&self, at: usize, len: usize) -> Result<&[u8], ZipError> {
        at.checked_add(len)
            .and_then(|end| self.0.get(at..end))
            .ok_or_else(|| ZipError::Malformed("it ends early".into()))
    }
    fn u16(&self, at: usize) -> Result<u16, ZipError> {
        let b = self.bytes(at, 2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&self, at: usize) -> Result<u32, ZipError> {
        let b = self.bytes(at, 4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
}

/// A project from a zip. Names must be paths inside a project; folders are
/// skipped. A zip of a project's folder, with everything under one top
/// folder holding `project.toml`, is read as that folder's contents.
pub fn import(zip: &[u8]) -> Result<MemoryStore, ZipError> {
    let r = Reader(zip);
    // The end record is in the last 22 bytes, plus up to 64 KiB of comment.
    let end = (0..=zip.len().saturating_sub(22))
        .rev()
        .take(22 + 65_535)
        .find(|&at| r.u32(at).ok() == Some(END))
        .ok_or_else(|| ZipError::Malformed("there's no end-of-directory record".into()))?;
    let count = usize::from(r.u16(end + 10)?);
    let directory = r.u32(end + 16)? as usize;
    if r.u32(end + 12)? == u32::MAX || directory == u32::MAX as usize || count == 0xffff {
        return Err(ZipError::Unsupported("it's a ZIP64 archive".into()));
    }

    let mut entries = Vec::with_capacity(count);
    let mut at = directory;
    let mut total = 0usize;
    for _ in 0..count {
        if r.u32(at)? != CENTRAL {
            return Err(ZipError::Malformed("its directory is damaged".into()));
        }
        let flags = r.u16(at + 8)?;
        let method = r.u16(at + 10)?;
        let crc = r.u32(at + 16)?;
        let compressed = r.u32(at + 20)? as usize;
        let size = r.u32(at + 24)? as usize;
        let name_len = usize::from(r.u16(at + 28)?);
        let extra_len = usize::from(r.u16(at + 30)?);
        let comment_len = usize::from(r.u16(at + 32)?);
        let local = r.u32(at + 42)? as usize;
        let name = String::from_utf8_lossy(r.bytes(at + 46, name_len)?).into_owned();
        at += 46 + name_len + extra_len + comment_len;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(ZipError::Unsupported(format!("{name} is encrypted")));
        }
        if method != 0 && method != 8 {
            return Err(ZipError::Unsupported(format!(
                "{name} uses compression method {method}"
            )));
        }
        total = total.saturating_add(size);
        if size > MAX_FILE || total > MAX_TOTAL {
            return Err(ZipError::TooLarge(name));
        }
        entries.push((name, method, crc, compressed, size, local));
    }

    // A project zipped as its folder: strip the one top folder.
    let prefix = entries.first().and_then(|(name, ..)| {
        let (top, _) = name.split_once('/')?;
        let prefix = format!("{top}/");
        (entries.iter().all(|(n, ..)| n.starts_with(&prefix))
            && entries
                .iter()
                .any(|(n, ..)| *n == format!("{prefix}project.toml")))
        .then_some(prefix)
    });

    let mut store = MemoryStore::new();
    for (name, method, crc, compressed, size, local) in entries {
        let inside = prefix
            .as_deref()
            .and_then(|p| name.strip_prefix(p))
            .unwrap_or(&name);
        let path = ProjectPath::new(inside).map_err(|_| ZipError::BadName(name.clone()))?;
        if r.u32(local)? != LOCAL {
            return Err(ZipError::Malformed(format!("{name}'s header is damaged")));
        }
        let start = local + 30 + usize::from(r.u16(local + 26)?) + usize::from(r.u16(local + 28)?);
        let data = r.bytes(start, compressed)?;
        let bytes = if method == 0 {
            data.to_vec()
        } else {
            decompress_to_vec_with_limit(data, size).map_err(|_| ZipError::Damaged(name.clone()))?
        };
        if bytes.len() != size || crc32(&bytes) != crc {
            return Err(ZipError::Damaged(name));
        }
        // A MemoryStore never fails to write.
        let _ = store.write(&path, &bytes);
    }
    Ok(store)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{ZipError, export, import};
    use crate::{MemoryStore, ProjectPath, Store};

    fn project() -> MemoryStore {
        let mut store = MemoryStore::new();
        let rinex: Vec<u8> = (0..20_000u32)
            .flat_map(|i| (i % 97).to_le_bytes())
            .collect();
        store
            .add_raw(&ProjectPath::new("raw/A.rnx").unwrap(), &rinex)
            .unwrap();
        for (path, text) in [
            ("project.toml", "schema = 1\nname = \"Test\"\n"),
            ("marks.toml", "schema = 1\n"),
            ("occupations/A.toml", "id = \"A\"\n"),
            ("fieldlogs/A-tape.jpg", "\u{ff}\u{d8}"),
            (".cache/ab/abcd", "derived"),
        ] {
            store
                .write(&ProjectPath::new(path).unwrap(), text.as_bytes())
                .unwrap();
        }
        store
    }

    #[test]
    fn projects_round_trip_without_their_cache() {
        let original = project();
        let zip = export(&original).unwrap();
        let back = import(&zip).unwrap();
        let mut expected = original.clone();
        expected = {
            let mut s = MemoryStore::new();
            for (p, b) in expected
                .files()
                .filter(|(p, _)| !p.as_str().starts_with(".cache/"))
            {
                s.write(p, b).unwrap();
            }
            s
        };
        assert_eq!(back, expected);
        // raw.lock came across, so the raw file still checks.
        assert!(
            back.read_raw(&ProjectPath::new("raw/A.rnx").unwrap())
                .is_ok()
        );
        // The same project always gives the same bytes.
        assert_eq!(export(&original).unwrap(), zip);
        // The RINEX-like file was deflated.
        assert!(zip.len() < 80_000 / 4);
    }

    #[test]
    fn hostile_and_damaged_zips_are_refused() {
        let mut evil = MemoryStore::new();
        evil.write(&ProjectPath::new("project.toml").unwrap(), b"x")
            .unwrap();
        let zip = export(&evil).unwrap();
        // Rename the entry, in both headers, to a path escaping the project.
        let mut escaped = zip.clone();
        let name = b"project.toml";
        for at in 0..escaped.len() - name.len() {
            if &escaped[at..at + name.len()] == name {
                escaped[at..at + name.len()].copy_from_slice(b"../etc/x.com");
            }
        }
        assert_eq!(
            import(&escaped),
            Err(ZipError::BadName("../etc/x.com".into()))
        );
        // Flip a byte of the stored data: the CRC catches it.
        let mut damaged = zip.clone();
        let at = 30 + "project.toml".len();
        damaged[at] ^= 0xff;
        assert_eq!(
            import(&damaged),
            Err(ZipError::Damaged("project.toml".into()))
        );
        assert!(matches!(import(b"not a zip"), Err(ZipError::Malformed(_))));
        assert!(matches!(
            import(&zip[..zip.len() - 3]),
            Err(ZipError::Malformed(_))
        ));
    }
}
