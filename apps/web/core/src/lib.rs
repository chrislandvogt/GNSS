//! The core compiled to WebAssembly, for the web app's core worker.
//!
//! This crate only adapts the `api` crate to JavaScript. It holds no domain
//! logic (ARCHITECTURE.md section 8). [`Session`] keeps the files the page
//! has loaded and the last adjustment, so exports don't adjust again; it
//! compiles natively too, so it's tested both ways. The bindings exist only
//! in the WebAssembly build.

use std::collections::BTreeMap;

use gnss_api::project::{self, ExportFile, ExportFormat, Format, Project, Run, Summary};
use gnss_datum::FrameId;
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

#[cfg(target_arch = "wasm32")]
mod bindings {
    use std::cell::RefCell;

    use wasm_bindgen::prelude::{JsError, JsValue, wasm_bindgen};

    use super::Session;

    thread_local! {
        static SESSION: RefCell<Session> = RefCell::new(Session::default());
    }

    fn json<T: serde::Serialize>(value: &T) -> Result<String, JsError> {
        serde_json::to_string(value).map_err(|e| JsError::new(&e.to_string()))
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

    use super::{Session, catalogue, project_toml};

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
