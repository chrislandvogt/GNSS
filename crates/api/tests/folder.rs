//! Loading an occupation-first project from a store (ARCHITECTURE.md
//! section 6): every problem is reported by file, and a file with problems is
//! left out while the rest loads.

#![allow(clippy::unwrap_used)]

use gnss_api::folder::{LoadError, load};
use gnss_api::project::{AdjustmentSettings, Project};
use gnss_datum::FrameId;
use gnss_store::{MemoryStore, ProjectPath, Store};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn path(p: &str) -> ProjectPath {
    ProjectPath::new(p).unwrap()
}

fn occupation(id: &str, mark: &str, rinex: &str) -> String {
    format!(
        "schema = 1\nid = \"{id}\"\nmark = \"{mark}\"\nstart = \"2026-10-14T00:12:00Z\"\nend = \"2026-10-14T02:31:30Z\"\n\
         rinex = [\"{rinex}\"]\n\
         [receiver]\nmake = \"Leica\"\nmodel = \"GS10\"\n\
         [antenna]\nigs = \"LEIAS10\"\n\
         [height]\nmethod = \"vertical\"\nto = \"ARP\"\nvalue = 1.532\nphotos = [\"fieldlogs/{id}-tape.jpg\"]\n"
    )
}

/// Two marks, each occupied once, with their RINEX and tape photos.
fn project() -> MemoryStore {
    let mut store = MemoryStore::new();
    let settings = Project::new("Test", AdjustmentSettings::new(FrameId::Gda2020, None));
    store
        .write(
            &path("project.toml"),
            settings.to_toml().unwrap().as_bytes(),
        )
        .unwrap();
    store
        .write(
            &path("marks.toml"),
            b"schema = 1\n[[mark]]\nid = \"PM1\"\n[[mark]]\nid = \"PM2\"\n",
        )
        .unwrap();
    for (id, mark) in [("A", "PM1"), ("B", "PM2")] {
        let rinex = format!("raw/{id}.rnx");
        store.add_raw(&path(&rinex), id.as_bytes()).unwrap();
        store
            .write(&path(&format!("fieldlogs/{id}-tape.jpg")), b"jpeg")
            .unwrap();
        store
            .write(
                &path(&format!("occupations/{id}.toml")),
                occupation(id, mark, &rinex).as_bytes(),
            )
            .unwrap();
    }
    store
}

fn issues(store: &MemoryStore) -> Vec<String> {
    load(store)
        .unwrap()
        .issues
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn a_good_project_loads_without_issues() {
    let loaded = load(&project()).unwrap();
    assert!(loaded.issues.is_empty(), "{:?}", loaded.issues);
    assert_eq!(loaded.marks.marks.len(), 2);
    let ids: Vec<&str> = loaded.occupations.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["A", "B"]);
    assert_eq!(loaded.project.name, "Test");
}

#[test]
fn without_project_toml_it_isnt_a_project() {
    assert_eq!(load(&MemoryStore::new()), Err(LoadError::NotAProject));
    let mut store = project();
    store.write(&path("project.toml"), b"schema = 1\n").unwrap();
    assert!(matches!(load(&store), Err(LoadError::Project(_))));
}

#[test]
fn every_problem_is_reported_and_the_rest_loads() {
    let mut store = project();
    // A's RINEX is edited after import; B's tape photo goes missing.
    store.write(&path("raw/A.rnx"), b"edited").unwrap();
    let b = occupation("B", "PM2", "raw/B.rnx").replace("B-tape.jpg", "B-tape2.jpg");
    store
        .write(&path("occupations/B.toml"), b.as_bytes())
        .unwrap();
    // C is in a file named for another id, D occupies an unknown mark and
    // E's file doesn't read; a stray file sits among the occupations.
    store
        .write(
            &path("occupations/C.toml"),
            occupation("C2", "PM1", "raw/A.rnx").as_bytes(),
        )
        .unwrap();
    store.add_raw(&path("raw/D.rnx"), b"D").unwrap();
    store.write(&path("fieldlogs/D-tape.jpg"), b"jpeg").unwrap();
    store
        .write(
            &path("occupations/D.toml"),
            occupation("D", "PM9", "raw/D.rnx").as_bytes(),
        )
        .unwrap();
    store
        .write(&path("occupations/E.toml"), b"schema = 1\nid = \"E\"\n")
        .unwrap();
    store.write(&path("occupations/notes.txt"), b"x").unwrap();

    let loaded = load(&store).unwrap();
    let ids: Vec<&str> = loaded.occupations.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["D"]);
    let found = issues(&store);
    let expected = [
        (
            "occupations/A.toml",
            "raw/A.rnx: it has changed since it was imported",
        ),
        ("occupations/B.toml", "fieldlogs/B-tape2.jpg: no such file"),
        ("occupations/C.toml", "its id is C2 but its file is C.toml"),
        ("occupations/E.toml", "missing field"),
        ("occupations/notes.txt", "only .toml files"),
        ("occupations/D.toml", "mark PM9 isn't in marks.toml"),
    ];
    assert_eq!(found.len(), expected.len(), "{found:#?}");
    for ((file, message), issue) in expected.iter().zip(&found) {
        assert!(
            issue.starts_with(&format!("{file}: ")) && issue.contains(message),
            "expected {file}: …{message}… got {issue}"
        );
    }
}

#[test]
fn a_broken_marks_file_is_an_issue_not_a_failure() {
    let mut store = project();
    store
        .write(&path("marks.toml"), b"schema = 1\n[[mark]]\nid = \"P M\"\n")
        .unwrap();
    let found = issues(&store);
    assert!(found[0].starts_with("marks.toml: "), "{found:?}");
    // Without the marks, both occupations' marks are unknown.
    assert_eq!(found.len(), 3, "{found:?}");
}

fn overlay(id: &str, file: &str) -> String {
    format!(
        "schema = 1\nid = \"{id}\"\nfile = \"{file}\"\nreason = \"Field log shows an AS10\"\nby = \"CL\"\n\
         recorded = \"2026-10-15T03:00:00Z\"\n[rinex]\nantenna = \"LEIAS10\"\n"
    )
}

#[test]
fn overlays_load_in_id_order_and_must_correct_a_locked_raw_file() {
    let mut store = project();
    for (name, id, file) in [
        ("b-later", "b-later", "raw/B.rnx"),
        ("a-first", "a-first", "raw/A.rnx"),
        ("c-unimported", "c-unimported", "raw/C.rnx"),
        ("d-misnamed", "d-other", "raw/A.rnx"),
    ] {
        store
            .write(
                &path(&format!("overlays/{name}.toml")),
                overlay(id, file).as_bytes(),
            )
            .unwrap();
    }
    let loaded = load(&store).unwrap();
    let ids: Vec<&str> = loaded.overlays.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["a-first", "b-later"]);
    let found: Vec<String> = loaded.issues.iter().map(ToString::to_string).collect();
    assert_eq!(found.len(), 2, "{found:#?}");
    assert!(
        found[0].starts_with("overlays/c-unimported.toml: raw/C.rnx: no such file"),
        "{found:?}"
    );
    assert!(
        found[1].contains("its id is d-other but its file is d-misnamed.toml"),
        "{found:?}"
    );

    // Editing the raw file it corrects is caught, as for an occupation.
    store.write(&path("raw/B.rnx"), b"edited").unwrap();
    let found = issues(&store);
    assert!(
        found
            .iter()
            .any(|i| i.starts_with("overlays/b-later.toml: raw/B.rnx: it has changed")),
        "{found:?}"
    );
}
