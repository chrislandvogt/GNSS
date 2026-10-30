//! Offline data packs (ARCHITECTURE.md section 13): files from the cache in
//! a zip, with a manifest of where each came from, its hash, its licence and
//! the attributions, so a machine with no network can use them.
//!
//! A pack holds `manifest.json` and each file at `files/<source>/<name>`.
//! Importing checks every file against its hash before it goes into the
//! cache, so a damaged or altered pack is refused, not half-used.

use gnss_store::{MemoryStore, ProjectPath, Store, sha256_hex, zip};
use serde::{Deserialize, Serialize};

use crate::FetchError;
use crate::cache::{Cache, Entry};

/// The manifest's name in a pack.
pub const MANIFEST: &str = "manifest.json";

/// The pack format, raised only when old readers couldn't read a new pack.
pub const FORMAT: u32 = 1;

/// What a pack holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The pack format: [`FORMAT`].
    pub format: u32,
    /// When it was made, `YYYY-MM-DDTHH:MM:SSZ`.
    pub created: String,
    /// What reports must say about the data, word for word.
    pub attributions: Vec<String>,
    /// The files, as the cache knows them.
    pub files: Vec<Entry>,
}

fn file_path(entry: &Entry) -> Result<ProjectPath, FetchError> {
    Ok(ProjectPath::new(format!(
        "files/{}/{}",
        entry.source, entry.name
    ))?)
}

/// A pack of cached files. Each must be in the cache and whole.
pub fn export<S: Store>(
    cache: &Cache<S>,
    files: &[Entry],
    attributions: &[String],
    created: &str,
) -> Result<Vec<u8>, FetchError> {
    let mut store = MemoryStore::new();
    for entry in files {
        let bytes = cache.read(entry)?.ok_or_else(|| {
            FetchError::DataPack(format!(
                "{} isn't in the cache, or is damaged; fetch it again",
                entry.name
            ))
        })?;
        store.write(&file_path(entry)?, &bytes)?;
    }
    let manifest = Manifest {
        format: FORMAT,
        created: created.to_owned(),
        attributions: attributions.to_vec(),
        files: files.to_vec(),
    };
    let json =
        serde_json::to_vec_pretty(&manifest).map_err(|e| FetchError::DataPack(e.to_string()))?;
    store.write(&ProjectPath::new(MANIFEST)?, &json)?;
    zip::export(&store).map_err(|e| FetchError::DataPack(e.to_string()))
}

/// Reads a pack into the cache, checking every file against its hash
/// first: all of it, or none. Returns its manifest.
pub fn import<S: Store>(cache: &mut Cache<S>, pack: &[u8]) -> Result<Manifest, FetchError> {
    let store = zip::import(pack).map_err(|e| FetchError::DataPack(e.to_string()))?;
    let json = store.read(&ProjectPath::new(MANIFEST)?)?.ok_or_else(|| {
        FetchError::DataPack(format!("it has no {MANIFEST}, so it isn't a data pack"))
    })?;
    let manifest: Manifest = serde_json::from_slice(&json)
        .map_err(|e| FetchError::DataPack(format!("{MANIFEST}: {e}")))?;
    if manifest.format > FORMAT {
        return Err(FetchError::DataPack(format!(
            "it's format {}, newer than this version reads ({FORMAT}); update the software",
            manifest.format
        )));
    }
    let mut checked = Vec::new();
    for entry in &manifest.files {
        let bytes = store.read(&file_path(entry)?)?.ok_or_else(|| {
            FetchError::DataPack(format!(
                "{} is in the manifest but not the pack",
                entry.name
            ))
        })?;
        if sha256_hex(&bytes) != entry.sha256 || bytes.len() as u64 != entry.bytes {
            return Err(FetchError::DataPack(format!(
                "{} doesn't match its hash in the manifest; the pack is damaged or was changed",
                entry.name
            )));
        }
        checked.push((entry, bytes));
    }
    for (entry, bytes) in checked {
        cache.put(entry.clone(), &bytes)?;
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::*;

    fn entry(name: &str) -> Entry {
        Entry {
            source: "ga-archive".into(),
            name: name.into(),
            sha256: String::new(),
            bytes: 0,
            url: format!("https://x.example/{name}"),
            fetched: "2026-10-29T00:00:00Z".into(),
            licence: "CC BY 4.0".into(),
        }
    }

    #[test]
    fn packs_carry_files_between_caches_checked() {
        let mut online = Cache::new(MemoryStore::new());
        let a = online.put(entry("a.crx.gz"), b"aaaa").unwrap();
        let b = online.put(entry("b.rnx.gz"), b"bb").unwrap();
        let attributions = vec!["GA data, CC BY 4.0".to_owned()];
        let pack = export(
            &online,
            &[a.clone(), b.clone()],
            &attributions,
            "2026-10-30T00:00:00Z",
        )
        .unwrap();

        let mut offline = Cache::new(MemoryStore::new());
        let manifest = import(&mut offline, &pack).unwrap();
        assert_eq!(manifest.attributions, attributions);
        assert_eq!(
            offline.get("ga-archive", "a.crx.gz").unwrap(),
            Some(a.clone())
        );
        assert_eq!(offline.read(&b).unwrap().unwrap(), b"bb");

        // A file that isn't cached can't be packed.
        assert!(export(&offline, &[entry("missing")], &[], "").is_err());
    }

    #[test]
    fn altered_packs_are_refused_whole() {
        // A pack whose second file doesn't match its manifest: nothing of
        // it is imported, not even the first file.
        let mut store = MemoryStore::new();
        store
            .write(
                &ProjectPath::new("files/ga-archive/a.crx.gz").unwrap(),
                b"aaaa",
            )
            .unwrap();
        store
            .write(
                &ProjectPath::new("files/ga-archive/b.rnx.gz").unwrap(),
                b"bX",
            )
            .unwrap();
        let mut first = entry("a.crx.gz");
        first.sha256 = sha256_hex(b"aaaa");
        first.bytes = 4;
        let mut second = entry("b.rnx.gz");
        second.sha256 = sha256_hex(b"bb");
        second.bytes = 2;
        let manifest = Manifest {
            format: FORMAT,
            created: String::new(),
            attributions: Vec::new(),
            files: vec![first, second],
        };
        store
            .write(
                &ProjectPath::new(MANIFEST).unwrap(),
                &serde_json::to_vec(&manifest).unwrap(),
            )
            .unwrap();
        let pack = zip::export(&store).unwrap();
        let mut offline = Cache::new(MemoryStore::new());
        let err = import(&mut offline, &pack).unwrap_err().to_string();
        assert!(err.contains("b.rnx.gz doesn't match its hash"), "{err}");
        assert_eq!(offline.get("ga-archive", "a.crx.gz").unwrap(), None);
        // Not a pack at all.
        let mut empty = MemoryStore::new();
        empty
            .write(&ProjectPath::new("x.txt").unwrap(), b"x")
            .unwrap();
        let err = import(&mut offline, &zip::export(&empty).unwrap())
            .unwrap_err()
            .to_string();
        assert!(err.contains("manifest.json"), "{err}");
    }
}
