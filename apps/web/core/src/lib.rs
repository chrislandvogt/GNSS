//! The core compiled to WebAssembly, for the web app's core worker.
//!
//! This crate only adapts the `api` crate to JavaScript. It holds no domain
//! logic (ARCHITECTURE.md section 8). [`Session`] keeps the files the page
//! has loaded and the last adjustment, so exports don't adjust again; it
//! compiles natively too, so it's tested both ways. The bindings exist only
//! in the WebAssembly build.

use std::collections::BTreeMap;

pub mod fetch;

use gnss_api::folder;
use gnss_api::project::{self, ExportFile, ExportFormat, Format, Project, Run, Summary};
use gnss_datum::FrameId;
use gnss_store::{MemoryStore, ProjectPath, Store, zip};
use serde::Serialize;

/// A loaded file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LoadedFile {
    /// Its name.
    pub name: String,
    /// Its size in bytes.
    pub bytes: usize,
    /// Its SHA-256.
    pub sha256: String,
    /// The format it looks like, if any.
    pub format: Option<&'static str>,
}

/// An adjustment's outcome.
#[derive(Debug, Clone, Serialize)]
pub struct Adjusted {
    /// The summary.
    pub summary: Summary,
    /// The HTML report.
    pub report: String,
}

/// What the page offers: frames, import formats and export formats.
#[derive(Debug, Clone, Serialize)]
pub struct Catalogue {
    /// Frames, in the order to offer them.
    pub frames: Vec<FrameEntry>,
    /// Import formats.
    pub formats: Vec<FormatEntry>,
    /// Export formats.
    pub exports: Vec<FormatEntry>,
}

/// A frame.
#[derive(Debug, Clone, Serialize)]
pub struct FrameEntry {
    /// Its name, such as "GDA2020".
    pub name: &'static str,
    /// Whether it's static (plate-fixed), so it needs no epoch.
    #[serde(rename = "static")]
    pub is_static: bool,
}

/// A file format.
#[derive(Debug, Clone, Serialize)]
pub struct FormatEntry {
    /// Its name in project files.
    pub name: &'static str,
    /// Its name for people.
    pub label: &'static str,
    /// For imports: whether its files always say which frame they're in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub states_frame: Option<bool>,
    /// For exports: what it holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'static str>,
}

/// The frames, formats and export formats the page offers.
pub fn catalogue() -> Catalogue {
    Catalogue {
        frames: FrameId::ALL
            .iter()
            .map(|f| FrameEntry {
                name: f.name(),
                is_static: f.is_static(),
            })
            .collect(),
        formats: Format::ALL
            .iter()
            .map(|&(f, name)| FormatEntry {
                name,
                label: f.label(),
                states_frame: Some(f.states_frame()),
                description: None,
            })
            .collect(),
        exports: ExportFormat::ALL
            .iter()
            .map(|&(f, name)| FormatEntry {
                name,
                label: f.label(),
                states_frame: None,
                description: Some(f.description()),
            })
            .collect(),
    }
}

/// The files a page has loaded, and its last adjustment.
#[derive(Debug, Default)]
pub struct Session {
    files: BTreeMap<String, Vec<u8>>,
    last: Option<Run>,
}

impl Session {
    /// Loads a file, replacing one of the same name.
    pub fn add_file(&mut self, name: &str, contents: Vec<u8>) -> LoadedFile {
        let loaded = LoadedFile {
            name: name.to_owned(),
            bytes: contents.len(),
            sha256: gnss_api::hash_file(&contents),
            format: project::guess_format(name, &contents).map(Format::name),
        };
        self.files.insert(name.to_owned(), contents);
        self.last = None;
        loaded
    }

    /// Forgets a file.
    pub fn remove_file(&mut self, name: &str) {
        self.files.remove(name);
        self.last = None;
    }

    /// Adjusts a project, given as JSON in `project.toml`'s shape, whose
    /// imports name loaded files. `generated` is the time to show in the
    /// report.
    pub fn adjust(&mut self, project_json: &str, generated: &str) -> Result<Adjusted, String> {
        // A failed adjustment leaves no results to export.
        self.last = None;
        let project = parse(project_json)?;
        let files = &self.files;
        let run = project::run(&project, &mut |import| {
            files
                .get(import.name())
                .cloned()
                .ok_or_else(|| "it isn't loaded".to_owned())
        })
        .map_err(|e| e.to_string())?;
        let adjusted = Adjusted {
            summary: project::summary(&project, &run),
            report: project::report(&project, &run, generated),
        };
        self.last = Some(run);
        Ok(adjusted)
    }

    /// The last adjustment's results in a format. `now` is the time to
    /// record, in ISO 8601 UTC.
    pub fn export(
        &self,
        format: &str,
        ellipse_scale: f64,
        now: &str,
    ) -> Result<Vec<ExportFile>, String> {
        let format = ExportFormat::parse(format)
            .ok_or_else(|| format!("there's no export format {format:?}"))?;
        let run = self
            .last
            .as_ref()
            .ok_or("adjust first: there are no results to export")?;
        project::export(run, format, ellipse_scale, now).map_err(|e| e.to_string())
    }
}

/// A project from JSON in `project.toml`'s shape.
fn parse(project_json: &str) -> Result<Project, String> {
    let project: Project =
        serde_json::from_str(project_json).map_err(|e| format!("the project: {e}"))?;
    if project.schema != project::SCHEMA {
        return Err(format!(
            "the project: schema {} isn't one this version reads ({})",
            project.schema,
            project::SCHEMA
        ));
    }
    Ok(project)
}

/// A project given as JSON, as `project.toml`'s text.
pub fn project_toml(project_json: &str) -> Result<String, String> {
    parse(project_json)?.to_toml().map_err(|e| e.to_string())
}

/// A file in a project, by its path relative to the project folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFile {
    /// Its path, such as `raw/A.rnx`.
    pub path: String,
    /// Its contents.
    pub data: Vec<u8>,
}

/// A store holding a project's files, each path checked.
fn store(files: Vec<ProjectFile>) -> Result<MemoryStore, String> {
    let mut store = MemoryStore::new();
    for file in files {
        let path = ProjectPath::new(&file.path).map_err(|e| e.to_string())?;
        store.write(&path, &file.data).map_err(|e| e.to_string())?;
    }
    Ok(store)
}

/// A project's files as a zip, the same bytes the CLI would make. The
/// derived-data cache is left out.
pub fn zip_export(files: Vec<ProjectFile>) -> Result<Vec<u8>, String> {
    zip::export(&store(files)?).map_err(|e| e.to_string())
}

/// The files in a project zip, checked: none escapes the project, and none is
/// damaged.
pub fn zip_import(zip: &[u8]) -> Result<Vec<ProjectFile>, String> {
    let store = zip::import(zip).map_err(|e| e.to_string())?;
    Ok(store
        .files()
        .map(|(path, data)| ProjectFile {
            path: path.to_string(),
            data: data.to_vec(),
        })
        .collect())
}

/// What checking a project found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Checked {
    /// The project's name.
    pub name: String,
    /// How many marks, occupations and overlays read without problems.
    pub marks: usize,
    /// See `marks`.
    pub occupations: usize,
    /// See `marks`.
    pub overlays: usize,
    /// Every problem, by file.
    pub issues: Vec<CheckedIssue>,
}

/// A problem in a project file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CheckedIssue {
    /// The file, relative to the project folder.
    pub file: String,
    /// What's wrong, and what to do about it.
    pub message: String,
}

/// Loads a project from its files the way the CLI does, raw files checked
/// against `raw.lock`, and reports what it found.
pub fn check_project(files: Vec<ProjectFile>) -> Result<Checked, String> {
    let loaded = folder::load(&store(files)?).map_err(|e| e.to_string())?;
    Ok(Checked {
        name: loaded.project.name,
        marks: loaded.marks.marks.len(),
        occupations: loaded.occupations.len(),
        overlays: loaded.overlays.len(),
        issues: loaded
            .issues
            .into_iter()
            .map(|i| CheckedIssue {
                file: i.file,
                message: i.message,
            })
            .collect(),
    })
}

/// How much of a file is read at a time: what the page reads from a `File`
/// in one go. A whole file is never in memory, only this much of it.
pub const SLICE: usize = 4 << 20;

/// A file read through `read(offset, length)`, which gives that part of it:
/// in the browser, a slice of a `File`.
pub struct Slices<F> {
    read: F,
    size: u64,
    pos: u64,
}

impl<F: FnMut(u64, usize) -> Result<Vec<u8>, String>> Slices<F> {
    /// A file of `size` bytes.
    pub fn new(size: u64, read: F) -> Self {
        Self { read, size, pos: 0 }
    }
}

impl<F: FnMut(u64, usize) -> Result<Vec<u8>, String>> std::io::Read for Slices<F> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.size.saturating_sub(self.pos);
        let want = usize::try_from(left).unwrap_or(usize::MAX).min(buf.len());
        if want == 0 {
            return Ok(0);
        }
        let bytes = (self.read)(self.pos, want).map_err(std::io::Error::other)?;
        let n = bytes.len().min(want);
        if n == 0 {
            return Err(std::io::Error::other("the file ended before its size"));
        }
        if let (Some(dst), Some(src)) = (buf.get_mut(..n), bytes.get(..n)) {
            dst.copy_from_slice(src);
        }
        self.pos += n as u64;
        Ok(n)
    }
}

/// Checks a RINEX observation file of `size` bytes read in slices through
/// `read`, with navigation files (given whole: they're small) for elevation
/// coverage.
pub fn check_rinex<F: FnMut(u64, usize) -> Result<Vec<u8>, String>>(
    size: u64,
    read: F,
    nav: &[Vec<u8>],
) -> Result<gnss_api::rinex::FileQc, String> {
    let mut ephemerides = Vec::new();
    for (i, bytes) in nav.iter().enumerate() {
        ephemerides.extend(
            gnss_api::rinex::read_nav(&bytes[..])
                .map_err(|e| format!("navigation file {}: {e}", i + 1))?,
        );
    }
    let orbits = (!nav.is_empty()).then(|| gnss_api::rinex::Orbits::new(ephemerides));
    let reader = std::io::BufReader::with_capacity(SLICE, Slices::new(size, read));
    gnss_api::rinex::check_file(reader, orbits).map_err(|e| e.to_string())
}

#[cfg(target_arch = "wasm32")]
mod bindings {
    use std::cell::RefCell;

    use wasm_bindgen::prelude::{JsError, JsValue, wasm_bindgen};

    use super::{ProjectFile, Session};

    thread_local! {
        static SESSION: RefCell<Session> = RefCell::new(Session::default());
    }

    fn json<T: serde::Serialize>(value: &T) -> Result<String, JsError> {
        serde_json::to_string(value).map_err(|e| JsError::new(&e.to_string()))
    }

    /// Checks a RINEX observation file of `size` bytes, read in slices by
    /// calling `read(offset, length)`, which returns a `Uint8Array`; `nav` is
    /// an array of navigation files' bytes. The report as JSON.
    #[wasm_bindgen(js_name = checkRinex)]
    pub fn check_rinex(
        size: f64,
        read: &js_sys::Function,
        nav: &js_sys::Array,
    ) -> Result<String, JsError> {
        let nav: Vec<Vec<u8>> = nav
            .iter()
            .map(|v| js_sys::Uint8Array::new(&v).to_vec())
            .collect();
        let slice = |offset: u64, length: usize| -> Result<Vec<u8>, String> {
            let bytes = read
                .call2(
                    &JsValue::NULL,
                    &JsValue::from_f64(offset as f64),
                    &JsValue::from_f64(length as f64),
                )
                .map_err(|e| format!("reading the file: {e:?}"))?;
            Ok(js_sys::Uint8Array::new(&bytes).to_vec())
        };
        let qc = super::check_rinex(size as u64, slice, &nav).map_err(|e| JsError::new(&e))?;
        json(&qc)
    }

    /// Version of the core.
    #[wasm_bindgen]
    pub fn version() -> String {
        gnss_api::VERSION.to_owned()
    }

    /// Which build of the core is running: always "web" here.
    #[wasm_bindgen(js_name = buildTarget)]
    pub fn build_target() -> String {
        gnss_api::BuildTarget::current().as_str().to_owned()
    }

    /// SHA-256 of a file's contents, as recorded in `raw.lock`.
    #[wasm_bindgen(js_name = hashFile)]
    pub fn hash_file(contents: &[u8]) -> String {
        gnss_api::hash_file(contents)
    }

    /// The frames, import formats and export formats, as JSON.
    #[wasm_bindgen]
    pub fn catalogue() -> Result<String, JsError> {
        json(&super::catalogue())
    }

    /// Loads a file; its details as JSON.
    #[wasm_bindgen(js_name = addFile)]
    pub fn add_file(name: &str, contents: Vec<u8>) -> Result<String, JsError> {
        json(&SESSION.with_borrow_mut(|s| s.add_file(name, contents)))
    }

    /// Forgets a file.
    #[wasm_bindgen(js_name = removeFile)]
    pub fn remove_file(name: &str) {
        SESSION.with_borrow_mut(|s| s.remove_file(name));
    }

    /// Adjusts a project given as JSON; the summary and report as JSON.
    #[wasm_bindgen]
    pub fn adjust(project: &str, generated: &str) -> Result<String, JsError> {
        let adjusted = SESSION
            .with_borrow_mut(|s| s.adjust(project, generated))
            .map_err(|e| JsError::new(&e))?;
        json(&adjusted)
    }

    /// The last adjustment's results in a format: an array of
    /// `{ name, data }`, with `data` a Uint8Array.
    #[wasm_bindgen(js_name = exportResults)]
    pub fn export_results(
        format: &str,
        ellipse_scale: f64,
        now: &str,
    ) -> Result<js_sys::Array, JsError> {
        let files = SESSION
            .with_borrow(|s| s.export(format, ellipse_scale, now))
            .map_err(|e| JsError::new(&e))?;
        let array = js_sys::Array::new();
        for file in files {
            let entry = js_sys::Object::new();
            js_sys::Reflect::set(&entry, &"name".into(), &file.name.into())
                .map_err(|_| JsError::new("couldn't build the export"))?;
            let data = js_sys::Uint8Array::from(file.bytes.as_slice());
            js_sys::Reflect::set(&entry, &"data".into(), &JsValue::from(data))
                .map_err(|_| JsError::new("couldn't build the export"))?;
            array.push(&entry);
        }
        Ok(array)
    }

    /// Files from JavaScript: an array of `{ path, data }`, with `data` a
    /// Uint8Array.
    fn files_from(array: &js_sys::Array) -> Result<Vec<ProjectFile>, JsError> {
        array
            .iter()
            .map(|entry| {
                let path = js_sys::Reflect::get(&entry, &"path".into())
                    .ok()
                    .and_then(|p| p.as_string())
                    .ok_or_else(|| JsError::new("a file has no path"))?;
                let data = js_sys::Reflect::get(&entry, &"data".into())
                    .map_err(|_| JsError::new("a file has no data"))?;
                Ok(ProjectFile {
                    path,
                    data: js_sys::Uint8Array::new(&data).to_vec(),
                })
            })
            .collect()
    }

    /// A project's files, as an array of `{ path, data }` for JavaScript.
    fn files_to(files: Vec<ProjectFile>) -> Result<js_sys::Array, JsError> {
        let array = js_sys::Array::new();
        for file in files {
            let entry = js_sys::Object::new();
            js_sys::Reflect::set(&entry, &"path".into(), &file.path.into())
                .map_err(|_| JsError::new("couldn't list the files"))?;
            let data = js_sys::Uint8Array::from(file.data.as_slice());
            js_sys::Reflect::set(&entry, &"data".into(), &JsValue::from(data))
                .map_err(|_| JsError::new("couldn't list the files"))?;
            array.push(&entry);
        }
        Ok(array)
    }

    /// HTTP through a page function: `get(url)` returns the body as a
    /// `Uint8Array`, `null` for a missing file, or throws.
    struct JsHttp<'a>(&'a js_sys::Function);

    impl gnss_api::fetch::gnss_fetch::Http for JsHttp<'_> {
        fn get(
            &mut self,
            url: &str,
        ) -> Result<Option<Vec<u8>>, gnss_api::fetch::gnss_fetch::FetchError> {
            match self.0.call1(&JsValue::NULL, &JsValue::from_str(url)) {
                Ok(body) if body.is_null() || body.is_undefined() => Ok(None),
                Ok(body) => Ok(Some(js_sys::Uint8Array::new(&body).to_vec())),
                Err(e) => Err(gnss_api::fetch::gnss_fetch::FetchError::Http {
                    url: gnss_api::fetch::gnss_fetch::cache::without_query(url).to_owned(),
                    message: js_sys::Error::from(e)
                        .message()
                        .as_string()
                        .unwrap_or_else(|| "the request failed".into()),
                }),
            }
        }
    }

    /// Fetches what the project's occupations need (`fetch::fetch_data`).
    /// `project` and `cache` are arrays of `{ path, data }` (the cache's
    /// objects may be empty: only their names are used); `options` is JSON;
    /// `get(url)` does the HTTP. Returns `[outcome JSON, cache files to
    /// save as { path, data }]`.
    #[wasm_bindgen(js_name = fetchData)]
    pub fn fetch_data(
        project: &js_sys::Array,
        cache: &js_sys::Array,
        options: &str,
        get: &js_sys::Function,
    ) -> Result<js_sys::Array, JsError> {
        let options: super::fetch::FetchOptions =
            serde_json::from_str(options).map_err(|e| JsError::new(&e.to_string()))?;
        let mut outcome = super::fetch::fetch_data(
            files_from(project)?,
            files_from(cache)?,
            &options,
            JsHttp(get),
        )
        .map_err(|e| JsError::new(&e))?;
        let writes = files_to(std::mem::take(&mut outcome.cache_writes))?;
        let out = js_sys::Array::new();
        out.push(&JsValue::from_str(&json(&outcome)?));
        out.push(&writes);
        Ok(out)
    }

    /// A project's files, as an array of `{ path, data }`, as a zip.
    #[wasm_bindgen(js_name = zipExport)]
    pub fn zip_export(files: &js_sys::Array) -> Result<Vec<u8>, JsError> {
        super::zip_export(files_from(files)?).map_err(|e| JsError::new(&e))
    }

    /// The files in a project zip, as an array of `{ path, data }`.
    #[wasm_bindgen(js_name = zipImport)]
    pub fn zip_import(zip: &[u8]) -> Result<js_sys::Array, JsError> {
        files_to(super::zip_import(zip).map_err(|e| JsError::new(&e))?)
    }

    /// Checks a project, given as an array of `{ path, data }`; what was
    /// found as JSON.
    #[wasm_bindgen(js_name = checkProject)]
    pub fn check_project(files: &js_sys::Array) -> Result<String, JsError> {
        let checked = super::check_project(files_from(files)?).map_err(|e| JsError::new(&e))?;
        json(&checked)
    }

    /// A project given as JSON, as `project.toml`'s text.
    #[wasm_bindgen(js_name = projectToml)]
    pub fn project_toml(project: &str) -> Result<String, JsError> {
        super::project_toml(project).map_err(|e| JsError::new(&e))
    }

    #[cfg(test)]
    mod tests {
        use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

        wasm_bindgen_test_configure!(run_in_browser);

        #[wasm_bindgen_test]
        fn reports_the_web_build() {
            assert_eq!(super::build_target(), "web");
        }

        #[wasm_bindgen_test]
        fn hashes_through_the_bindings() {
            assert_eq!(
                super::hash_file(b"abc"),
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{
        ProjectFile, SLICE, Session, catalogue, check_project, check_rinex, project_toml,
        zip_export, zip_import,
    };

    const SWTC: &[u8] = include_bytes!(
        "../../../../crates/rinex-io/tests/data/rinex/SWTC00AUS_20251660000.rnx.crx"
    );
    const HOB2_NAV: &[u8] = include_bytes!(
        "../../../../crates/rinex-io/tests/data/nav/HOB200AUS_R_20251660000_excerpt_MN.rnx"
    );

    #[test]
    fn rinex_is_read_in_slices_with_the_same_result_as_whole() {
        let whole = gnss_api::rinex::check_file(SWTC, None).unwrap();
        let mut calls = Vec::new();
        let sliced = check_rinex(
            SWTC.len() as u64,
            |offset, length| {
                calls.push((offset, length));
                let start = offset as usize;
                Ok(SWTC[start..start + length].to_vec())
            },
            &[],
        )
        .unwrap();
        assert_eq!(sliced, whole);
        // Read once, in order, in slices no bigger than SLICE.
        assert_eq!(calls.first(), Some(&(0, SWTC.len().min(SLICE))));
        assert!(calls.iter().all(|&(_, n)| n <= SLICE));
        // With a navigation file, coverage.
        let with_nav = check_rinex(
            SWTC.len() as u64,
            |o, n| Ok(SWTC[o as usize..o as usize + n].to_vec()),
            &[HOB2_NAV.to_vec()],
        )
        .unwrap();
        assert!(with_nav.systems.iter().any(|s| s.coverage.is_some()));
        // A short read, and a reader that fails, are errors, not panics.
        let short = check_rinex(
            SWTC.len() as u64 + 10,
            |o, n| {
                let start = (o as usize).min(SWTC.len());
                Ok(SWTC[start..(start + n).min(SWTC.len())].to_vec())
            },
            &[],
        );
        assert!(short.is_err());
        assert!(check_rinex(100, |_, _| Err("gone".into()), &[]).is_err());
    }

    const STATIONS: &[u8] =
        include_bytes!("../../../../crates/formats/tests/data/dynaml/gnss-networkstn.xml");
    const MEASUREMENTS: &[u8] =
        include_bytes!("../../../../crates/formats/tests/data/dynaml/gnss-networkmsr.xml");

    const PROJECT: &str = r#"{
        "schema": 1,
        "name": "Sample",
        "adjustment": { "frame": "GDA2020", "datum": "constrained", "uncertainty": "a-priori" },
        "import": [
            { "file": "raw/gnss-networkstn.xml", "format": "dynaml" },
            { "file": "raw/gnss-networkmsr.xml", "format": "dynaml" }
        ]
    }"#;

    #[test]
    fn adjusts_and_exports_loaded_files() {
        let mut session = Session::default();
        let loaded = session.add_file("gnss-networkstn.xml", STATIONS.to_vec());
        assert_eq!(loaded.format, Some("dynaml"));
        assert_eq!(loaded.bytes, STATIONS.len());
        assert!(session.export("gpkg", 1000.0, "x").is_err());
        let err = session.adjust(PROJECT, "now").unwrap_err();
        assert_eq!(err, "raw/gnss-networkmsr.xml: it isn't loaded");
        assert!(session.export("gpkg", 1000.0, "x").is_err());

        session.add_file("gnss-networkmsr.xml", MEASUREMENTS.to_vec());
        let adjusted = session.adjust(PROJECT, "now").unwrap();
        assert_eq!(adjusted.summary.degrees_of_freedom, 288);
        assert_eq!(adjusted.summary.suspects.len(), 12);
        assert!(adjusted.report.contains("<h1>Sample</h1>"));
        let files = session
            .export("csv", 1000.0, "2026-10-20T08:00:00.000Z")
            .unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["marks.csv", "observations.csv"]);
        assert!(session.export("shapefile", 1000.0, "x").is_err());

        // Loading or forgetting a file drops the results.
        session.remove_file("gnss-networkmsr.xml");
        assert!(session.export("csv", 1000.0, "x").is_err());

        // Edits arrive with the project.
        session.add_file("gnss-networkmsr.xml", MEASUREMENTS.to_vec());
        let edited = PROJECT.replace(
            "\n    }",
            r#",
        "edit": [{ "observation": "gnss-networkmsr.xml:435", "action": "exclude", "reason": "re-observed" }]
    }"#,
        );
        let adjusted = session.adjust(&edited, "now").unwrap();
        assert_eq!(adjusted.summary.degrees_of_freedom, 285);
        // A failed adjustment drops the last one's results.
        assert!(session.adjust("{}", "now").is_err());
        assert!(session.export("csv", 1000.0, "x").is_err());
    }

    #[test]
    fn projects_are_checked_and_written_as_toml() {
        let text = project_toml(PROJECT).unwrap();
        assert!(text.contains("[[import]]\nfile = \"raw/gnss-networkstn.xml\""));
        assert!(text.contains("alpha0 = 0.001"));
        let err = project_toml(&PROJECT.replace("\"schema\": 1", "\"schema\": 2")).unwrap_err();
        assert!(err.contains("schema 2"), "{err}");
        let err = project_toml(&PROJECT.replace("\"datum\"", "\"datun\"")).unwrap_err();
        assert!(err.contains("unknown field `datun`"), "{err}");
    }

    fn file(path: &str, data: &[u8]) -> ProjectFile {
        ProjectFile {
            path: path.into(),
            data: data.to_vec(),
        }
    }

    #[test]
    fn projects_go_in_and_out_of_zips_and_are_checked() {
        let project = "schema = 1\nname = \"Zipped\"\n\n[adjustment]\nframe = \"GDA2020\"\n";
        let files = vec![
            file("project.toml", project.as_bytes()),
            file("marks.toml", b"schema = 1\n"),
            file("occupations/bad.toml", b"not toml ["),
            file(".cache/ab/abcd", b"derived"),
        ];
        let zip = zip_export(files.clone()).unwrap();
        let back = zip_import(&zip).unwrap();
        let paths: Vec<&str> = back.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            ["marks.toml", "occupations/bad.toml", "project.toml"]
        );
        assert_eq!(back[2], files[0]);

        let checked = check_project(back).unwrap();
        assert_eq!(checked.name, "Zipped");
        assert_eq!((checked.marks, checked.occupations), (0, 0));
        assert_eq!(checked.issues.len(), 1);
        assert_eq!(checked.issues[0].file, "occupations/bad.toml");

        let err = zip_export(vec![file("../escape", b"x")]).unwrap_err();
        assert!(err.contains("../escape"), "{err}");
        assert!(zip_import(b"not a zip").is_err());
        let err = check_project(vec![file("marks.toml", b"")]).unwrap_err();
        assert!(err.contains("project.toml"), "{err}");
    }

    #[test]
    fn the_catalogue_lists_what_the_page_offers() {
        let c = catalogue();
        assert!(c.frames.iter().any(|f| f.name == "GDA2020" && f.is_static));
        assert!(
            c.frames
                .iter()
                .any(|f| f.name == "ITRF2014" && !f.is_static)
        );
        assert_eq!(c.formats.len(), 7);
        assert_eq!(c.exports[0].name, "gpkg");
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"static\":true"));
        assert!(json.contains("\"states_frame\":true"));
    }
}
