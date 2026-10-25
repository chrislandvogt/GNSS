//! The pipeline on a toy survey shaped like the real one: each occupation is
//! normalised, each pair of occupations gives a baseline, and the baselines
//! are adjusted together. M5's exit criterion: changing one antenna height
//! re-runs only what depends on it, and `verify` reproduces the rest.

#![allow(clippy::unwrap_used)]

use std::cell::{Cell, RefCell};

use gnss_pipeline::{
    Event, Graph, Hooks, Input, Key, Node, NodeId, RunError, Status, StoreCache, run, verify,
};
use gnss_store::MemoryStore;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// The graph for occupations A, B and C, each given as its file's text.
fn survey(files: &[(&str, &str)]) -> Graph {
    let mut nodes = Vec::new();
    for (id, text) in files {
        nodes.push(Node {
            id: NodeId::new(format!("normalise/{id}")),
            tool: "normalise 1".into(),
            settings: Vec::new(),
            inputs: vec![Input::Data {
                name: format!("occupations/{id}.toml"),
                hash: Key::of(text.as_bytes()),
            }],
        });
    }
    let mut baselines = Vec::new();
    for (i, (a, _)) in files.iter().enumerate() {
        for (b, _) in &files[i + 1..] {
            let id = NodeId::new(format!("baseline/{a}~{b}"));
            nodes.push(Node {
                id: id.clone(),
                tool: "engine 1".into(),
                settings: b"elevation mask = 15".to_vec(),
                inputs: vec![
                    Input::Node(NodeId::new(format!("normalise/{a}"))),
                    Input::Node(NodeId::new(format!("normalise/{b}"))),
                ],
            });
            baselines.push(Input::Node(id));
        }
    }
    nodes.push(Node {
        id: NodeId::new("adjust"),
        tool: "adjust 1".into(),
        settings: Vec::new(),
        inputs: baselines,
    });
    Graph::new(nodes).unwrap()
}

/// Each node's "work": its id and its inputs, joined.
fn compute(node: &Node, upstream: &[&[u8]]) -> Result<Vec<u8>, String> {
    let mut out = node.id.as_str().as_bytes().to_vec();
    for input in &node.inputs {
        if let Input::Data { hash, .. } = input {
            out.extend_from_slice(hash.hex().as_bytes());
        }
    }
    for bytes in upstream {
        out.push(b'|');
        out.extend_from_slice(bytes);
    }
    Ok(out)
}

fn computed(runs: &[gnss_pipeline::NodeRun]) -> Vec<&str> {
    runs.iter()
        .filter(|r| r.status == Status::Computed)
        .map(|r| r.id.as_str())
        .collect()
}

const DAY: [(&str, &str); 3] = [
    ("A", "height = 1.532"),
    ("B", "height = 1.604"),
    ("C", "height = 1.498"),
];

#[test]
fn changing_one_height_reruns_only_what_depends_on_it() {
    let mut store = MemoryStore::new();
    let first = run(
        &survey(&DAY),
        &mut StoreCache(&mut store),
        &mut compute,
        &mut Hooks::none(),
    )
    .unwrap();
    assert_eq!(computed(&first).len(), 7);

    // Nothing changed: nothing runs.
    let again = run(
        &survey(&DAY),
        &mut StoreCache(&mut store),
        &mut compute,
        &mut Hooks::none(),
    )
    .unwrap();
    assert!(computed(&again).is_empty());
    assert_eq!(
        again,
        first
            .iter()
            .map(|r| gnss_pipeline::NodeRun {
                status: Status::Cached,
                ..r.clone()
            })
            .collect::<Vec<_>>()
    );

    // B's antenna height is corrected: B, its two baselines and the adjustment
    // re-run, in order (each after its inputs; ties by id).
    let mut corrected = DAY;
    corrected[1].1 = "height = 1.640";
    let runs = run(
        &survey(&corrected),
        &mut StoreCache(&mut store),
        &mut compute,
        &mut Hooks::none(),
    )
    .unwrap();
    assert_eq!(
        computed(&runs),
        ["normalise/B", "baseline/A~B", "baseline/B~C", "adjust"]
    );

    // Changing it back reuses what was cached before.
    let back = run(
        &survey(&DAY),
        &mut StoreCache(&mut store),
        &mut compute,
        &mut Hooks::none(),
    )
    .unwrap();
    assert!(computed(&back).is_empty());
}

#[test]
fn verify_reproduces_everything_or_says_what_differs() {
    let mut store = MemoryStore::new();
    let graph = survey(&DAY);
    run(
        &graph,
        &mut StoreCache(&mut store),
        &mut compute,
        &mut Hooks::none(),
    )
    .unwrap();
    let before = store.clone();
    assert!(
        verify(
            &graph,
            &StoreCache(&mut store),
            &mut compute,
            &mut Hooks::none()
        )
        .unwrap()
        .is_empty()
    );
    assert_eq!(store, before, "verify doesn't change the cache");

    // A tool that no longer gives the same output for B's baselines.
    let mut drifted = |node: &Node, upstream: &[&[u8]]| {
        let mut out = compute(node, upstream)?;
        if node.id.as_str() == "baseline/B~C" {
            out.push(b'!');
        }
        Ok::<_, String>(out)
    };
    let found = verify(
        &graph,
        &StoreCache(&mut store),
        &mut drifted,
        &mut Hooks::none(),
    )
    .unwrap();
    let ids: Vec<&str> = found.iter().map(|m| m.id.as_str()).collect();
    // The drift shows in the baseline and in the adjustment that uses it.
    assert_eq!(ids, ["baseline/B~C", "adjust"]);
    assert!(
        found
            .iter()
            .all(|m| m.cached.is_some() && m.cached != Some(m.computed))
    );
}

#[test]
fn progress_cancellation_and_failures() {
    let mut store = MemoryStore::new();
    let graph = survey(&DAY);
    let events = RefCell::new(Vec::new());
    let done = Cell::new(0);
    // Cancel after three nodes have finished.
    let mut hooks = Hooks::new(
        |event| match event {
            Event::Started { node, index, total } => {
                events.borrow_mut().push(format!("{index}/{total} {node}"));
            }
            Event::Finished(_) => done.set(done.get() + 1),
        },
        || done.get() >= 3,
    );
    let result = run(
        &graph,
        &mut StoreCache(&mut store),
        &mut compute,
        &mut hooks,
    );
    assert_eq!(result, Err(RunError::Cancelled));
    assert_eq!(
        *events.borrow(),
        ["0/7 normalise/A", "1/7 normalise/B", "2/7 baseline/A~B"]
    );
    // What finished before the cancel is cached.
    let runs = run(
        &graph,
        &mut StoreCache(&mut store),
        &mut compute,
        &mut Hooks::none(),
    )
    .unwrap();
    assert_eq!(computed(&runs).len(), 4);

    // A failing node stops the run and is named.
    let mut failing = |node: &Node, upstream: &[&[u8]]| {
        if node.id.as_str() == "adjust" {
            return Err("the network has no datum".to_string());
        }
        compute(node, upstream)
    };
    let err = run(
        &survey(&[("A", "x"), ("B", "y")]),
        &mut StoreCache(&mut MemoryStore::new()),
        &mut failing,
        &mut Hooks::none(),
    )
    .unwrap_err();
    assert_eq!(err.to_string(), "adjust: the network has no datum");
}
