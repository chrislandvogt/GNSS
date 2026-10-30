//! The download cache, shared by every project (ARCHITECTURE.md section 13).
//!
//! It's kept in any [`Store`]: a folder in the user's cache directory
//! natively, and browser storage in the web app. Files are kept by content,
//! `objects/<first two hex digits>/<sha256>`, and found by source and name,
//! `index/<source>/<name>.json`, which records where and when each came
//! from. A file is kept exactly as published (compressed or not), so its
//! hash can be checked against the source's.

use gnss_store::{ProjectPath, Store, StoreError, sha256_hex};
use serde::{Deserialize, Serialize};

/// What the cache knows about a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The source's id in its pack.
    pub source: String,
    /// The file's name.
    pub name: String,
    /// The file's SHA-256, in hex.
    pub sha256: String,
    /// Its size.
    pub bytes: u64,
    /// Where it came from, without any signature in the query.
    pub url: String,
    /// When it was downloaded, `YYYY-MM-DDTHH:MM:SSZ`.
    pub fetched: String,
    /// The data's licence, from its source.
    pub licence: String,
}

/// A cache in a store.
pub struct Cache<S: Store> {
    store: S,
}

impl<S: Store> Cache<S> {
    /// A cache kept in `store`.
    pub fn new(store: S) -> Self {
        Self { store }
    }

    /// The store underneath.
    pub fn store(&self) -> &S {
        &self.store
    }

    /// The entry for a source's file, if it's cached and its object is there.
    pub fn get(&self, source: &str, name: &str) -> Result<Option<Entry>, StoreError> {
        let Some(bytes) = self.store.read(&index_path(source, name)?)? else {
            return Ok(None);
        };
        let Ok(entry) = serde_json::from_slice::<Entry>(&bytes) else {
            // A damaged index entry is a cache miss, not an error.
            return Ok(None);
        };
        Ok(self.has_object(&entry.sha256)?.then_some(entry))
    }

    /// Whether an object is kept, without reading it: a listing of its
    /// folder, not the file, which can be tens of megabytes.
    fn has_object(&self, sha256: &str) -> Result<bool, StoreError> {
        let object = object_path(sha256)?;
        let dir = object.as_str().rsplit_once('/').map_or("", |(d, _)| d);
        Ok(self.store.list(dir)?.contains(&object))
    }

    /// A cached file's bytes, checked against its hash. A damaged object is
    /// reported as missing, so it's downloaded again.
    pub fn read(&self, entry: &Entry) -> Result<Option<Vec<u8>>, StoreError> {
        let Some(bytes) = self.store.read(&object_path(&entry.sha256)?)? else {
            return Ok(None);
        };
        Ok((sha256_hex(&bytes) == entry.sha256).then_some(bytes))
    }

    /// Adds a file. The entry's `sha256` and `bytes` are filled in here.
    pub fn put(&mut self, mut entry: Entry, data: &[u8]) -> Result<Entry, StoreError> {
        entry.sha256 = sha256_hex(data);
        entry.bytes = data.len() as u64;
        let object = object_path(&entry.sha256)?;
        if !self.has_object(&entry.sha256)? {
            self.store.write(&object, data)?;
        }
        let index = index_path(&entry.source, &entry.name)?;
        let json = serde_json::to_vec_pretty(&entry).map_err(|e| StoreError::Io {
            path: index.to_string(),
            message: e.to_string(),
        })?;
        self.store.write(&index, &json)?;
        Ok(entry)
    }
}

/// Where an object is kept.
pub fn object_path(sha256: &str) -> Result<ProjectPath, StoreError> {
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(StoreError::BadPath(sha256.to_owned()));
    }
    ProjectPath::new(format!("objects/{}/{sha256}", &sha256[..2]))
}

fn index_path(source: &str, name: &str) -> Result<ProjectPath, StoreError> {
    let ok = |s: &str| !s.is_empty() && !s.starts_with('.') && !s.contains(['/', '\\', ':']);
    if !ok(source) || !ok(name) {
        return Err(StoreError::BadPath(format!("{source}/{name}")));
    }
    ProjectPath::new(format!("index/{source}/{name}.json"))
}

/// A URL without its query, which for signed links holds the signature.
pub fn without_query(url: &str) -> &str {
    url.split('?').next().unwrap_or(url)
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use gnss_store::MemoryStore;

    use super::*;

    fn entry() -> Entry {
        Entry {
            source: "ga-archive".into(),
            name: "SYDN00AUS_R_20251660000_01D_30S_MO.crx.gz".into(),
            sha256: String::new(),
            bytes: 0,
            url: without_query("https://x.example/SYDN.crx.gz?sig=secret").into(),
            fetched: "2026-10-29T00:00:00Z".into(),
            licence: "CC BY 4.0".into(),
        }
    }

    #[test]
    fn files_are_kept_by_content_and_found_by_name() {
        let mut cache = Cache::new(MemoryStore::default());
        assert_eq!(cache.get("ga-archive", &entry().name).unwrap(), None);
        let put = cache.put(entry(), b"data").unwrap();
        assert_eq!(put.bytes, 4);
        assert_eq!(put.url, "https://x.example/SYDN.crx.gz");
        let got = cache.get("ga-archive", &entry().name).unwrap().unwrap();
        assert_eq!(got, put);
        assert_eq!(cache.read(&got).unwrap().unwrap(), b"data");
        // A damaged object reads as missing.
        let object = object_path(&got.sha256).unwrap();
        let mut store = cache.store;
        store.write(&object, b"dat4").unwrap();
        let cache = Cache::new(store);
        assert_eq!(cache.read(&got).unwrap(), None);
        assert!(index_path("ga-archive", "../x").is_err());
        assert!(object_path("abc").is_err());
    }
}
