//! The JSON Schemas in the repository's `schemas/` folder match the project
//! files' types. After changing a type, regenerate them with
//! `GNSS_UPDATE_SCHEMAS=1 cargo test -p gnss-api --test schemas` and review
//! the diff. Native only: it reads the repository.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use gnss_api::folder::json_schemas;

#[test]
fn schemas_are_up_to_date() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let update = std::env::var_os("GNSS_UPDATE_SCHEMAS").is_some();
    if update {
        std::fs::create_dir_all(&dir).unwrap();
    }
    for (name, schema) in json_schemas() {
        let path = dir.join(name);
        if update {
            std::fs::write(&path, &schema).unwrap();
            continue;
        }
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            committed == schema,
            "schemas/{name} is out of date; regenerate it with \
             GNSS_UPDATE_SCHEMAS=1 cargo test -p gnss-api --test schemas"
        );
    }
}

/// The documented example, and the project files the tests write, are valid
/// against the schemas: a cheap check that the schemas describe the files.
#[test]
fn schemas_name_the_fields_the_files_use() {
    let schemas: Vec<(&str, serde_json::Value)> = json_schemas()
        .into_iter()
        .map(|(n, s)| (n, serde_json::from_str(&s).unwrap()))
        .collect();
    let properties = |name: &str| -> Vec<String> {
        let schema = &schemas.iter().find(|(n, _)| *n == name).unwrap().1;
        let mut keys: Vec<String> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(
        properties("occupation.schema.json"),
        [
            "antenna",
            "end",
            "field_log",
            "height",
            "id",
            "mark",
            "receiver",
            "rinex",
            "schema",
            "start"
        ]
    );
    assert_eq!(properties("marks.schema.json"), ["mark", "schema"]);
    assert_eq!(
        properties("project.schema.json"),
        ["adjustment", "edit", "import", "name", "schema"]
    );
    // deny_unknown_fields: editors flag a misspelt key.
    for (name, schema) in &schemas {
        assert_eq!(schema["additionalProperties"], false, "{name}");
    }
}
