//! The Australian jurisdiction pack (`packs/au/pack.toml`) against this crate:
//! every frame, transformation and height model it names must exist here,
//! with the same EPSG code, and every grid file must be fully described.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;

use gnss_datum::FrameId;
use gnss_datum::transform::{Method, builtin_transformations};
use toml::{Table, Value};
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;

#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

fn pack() -> Table {
    include_str!("../../../packs/au/pack.toml").parse().unwrap()
}

fn tables<'a>(pack: &'a Table, key: &str) -> Vec<&'a Table> {
    pack[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_table().unwrap())
        .collect()
}

fn text<'a>(table: &'a Table, key: &str) -> &'a str {
    table
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{key} is missing from {table:?}"))
}

#[test]
fn frames_are_known() {
    let pack = pack();
    let frames = pack["frames"].as_table().unwrap();
    for key in ["target", "adjustment"] {
        FrameId::parse(text(frames, key)).unwrap();
    }
    let supported: BTreeSet<FrameId> = frames["supported"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| FrameId::parse(v.as_str().unwrap()).unwrap())
        .collect();
    assert_eq!(supported, FrameId::ALL.into_iter().collect());
}

#[test]
fn transformations_are_the_ones_implemented() {
    let pack = pack();
    let builtin = builtin_transformations();
    let listed = tables(&pack, "transformations");
    let grids: BTreeSet<&str> = tables(&pack, "grids")
        .iter()
        .map(|g| text(g, "id"))
        .collect();

    for t in &listed {
        let name = text(t, "name");
        let epsg = t.get("epsg").map(|v| v.as_integer().unwrap());
        if let Some(grid) = t.get("grid") {
            // Grid transformations are built from their grid at run time.
            assert_eq!(name, "GDA94 to GDA2020 (2)");
            assert_eq!(epsg, Some(8447));
            assert!(
                grids.contains(grid.as_str().unwrap()),
                "{name}: no grid {grid}"
            );
            continue;
        }
        let ours = builtin
            .iter()
            .find(|b| b.name == name)
            .unwrap_or_else(|| panic!("the pack lists {name}, which the crate doesn't have"));
        assert!(!matches!(ours.method, Method::Grid(_)));
        match epsg {
            Some(code) => assert!(
                ours.source.contains(&format!("EPSG:{code}")),
                "{name}: the pack says EPSG:{code}, the crate says {}",
                ours.source
            ),
            None => assert!(!ours.source.starts_with("EPSG:"), "{name} has an EPSG code"),
        }
    }
    // And the crate has nothing the pack leaves out.
    for b in &builtin {
        assert!(
            listed.iter().any(|t| text(t, "name") == b.name),
            "the pack doesn't list {}",
            b.name
        );
    }
}

#[test]
fn height_models_and_grids_are_complete() {
    let pack = pack();
    let grids = tables(&pack, "grids");
    let ids: BTreeSet<&str> = grids.iter().map(|g| text(g, "id")).collect();
    assert_eq!(ids.len(), grids.len(), "grid ids must be unique");

    for m in tables(&pack, "height_models") {
        assert_eq!(text(m, "name"), "AUSGeoid2020");
        assert_eq!(FrameId::parse(text(m, "frame")).unwrap(), FrameId::Gda2020);
        assert_eq!(text(m, "height"), "AHD");
        for key in ["grid", "uncertainty"] {
            assert!(
                ids.contains(text(m, key)),
                "{key} {} isn't a grid",
                text(m, key)
            );
        }
    }

    let mut files = BTreeSet::new();
    for g in grids {
        let kind = text(g, "kind");
        assert!(["horizontal-shift", "geoid", "geoid-uncertainty"].contains(&kind));
        assert!(!text(g, "licence").is_empty() && !text(g, "description").is_empty());
        for f in g["files"].as_array().unwrap() {
            let f = f.as_table().unwrap();
            let file = text(f, "file");
            assert!(files.insert(file), "{file} is listed twice");
            let format = text(f, "format");
            let extension = if format == "ntv2" { ".gsb" } else { ".tif" };
            assert!(["ntv2", "geotiff"].contains(&format) && file.ends_with(extension));
            assert!(text(f, "url").starts_with("https://") && text(f, "url").ends_with(file));
            let sha256 = text(f, "sha256");
            assert!(
                sha256.len() == 64
                    && sha256
                        .chars()
                        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
                "{file}: bad SHA-256 {sha256}"
            );
            assert!(f["bytes"].as_integer().unwrap() > 0);
            assert!(["direct", "relay"].contains(&text(f, "browser")));
        }
    }
}
