//! Jobs: progress, issues and outcomes as the front ends receive them.

#![allow(clippy::unwrap_used)]

use std::cell::Cell;

use gnss_api::jobs::{JobEvent, JobId, Jobs, Outcome, run_graph};
use gnss_model::Issue;
use gnss_pipeline::{Graph, Input, Key, Node, NodeId, StoreCache};
use gnss_store::MemoryStore;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen_test::wasm_bindgen_test as test;
#[cfg(target_arch = "wasm32")]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

/// Two occupations and the baseline between them.
fn graph() -> Graph {
    let occupation = |id: &str| Node {
        id: NodeId::new(format!("normalise/{id}")),
        tool: "normalise 1".into(),
        settings: Vec::new(),
        inputs: vec![Input::Data {
            name: format!("occupations/{id}.toml"),
            hash: Key::of(id.as_bytes()),
        }],
    };
    Graph::new(vec![
        occupation("A"),
        occupation("B"),
        Node {
            id: NodeId::new("baseline/A~B"),
            tool: "engine 1".into(),
            settings: Vec::new(),
            inputs: vec![
                Input::Node(NodeId::new("normalise/A")),
                Input::Node(NodeId::new("normalise/B")),
            ],
        },
    ])
    .unwrap()
}

fn work(node: &Node, _: &[&[u8]]) -> Result<Vec<u8>, String> {
    if node.tool == "broken 1" {
        return Err("no overlap".into());
    }
    Ok(node.id.as_str().as_bytes().to_vec())
}

#[test]
fn a_job_reports_each_step_and_how_it_ended() {
    let mut jobs = Jobs::default();
    let job = jobs.start();
    assert_eq!(job, JobId(1));
    let mut store = MemoryStore::new();
    let mut events = Vec::new();
    let runs = run_graph(
        job,
        &graph(),
        &mut StoreCache(&mut store),
        &mut work,
        &mut |e| events.push(e),
        &|| false,
        None,
    );
    assert_eq!(runs.unwrap().len(), 3);
    let json: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect();
    assert_eq!(
        json,
        [
            r#"{"type":"progress","job":1,"done":0,"total":3,"step":"normalise/A"}"#,
            r#"{"type":"progress","job":1,"done":1,"total":3,"step":"normalise/B"}"#,
            r#"{"type":"progress","job":1,"done":2,"total":3,"step":"baseline/A~B"}"#,
            r#"{"type":"finished","job":1,"outcome":{"status":"succeeded","computed":3,"cached":0}}"#,
        ]
    );

    // Again: everything is reused.
    let job = jobs.start();
    let mut last = None;
    run_graph(
        job,
        &graph(),
        &mut StoreCache(&mut store),
        &mut work,
        &mut |e| last = Some(e),
        &|| false,
        None,
    );
    assert_eq!(
        last,
        Some(JobEvent::Finished {
            job: JobId(2),
            outcome: Outcome::Succeeded {
                computed: 0,
                cached: 3
            }
        })
    );
}

#[test]
fn failures_cancellation_and_issues_are_events_too() {
    let mut events = Vec::new();
    // The baseline's tool fails.
    let broken = Graph::new(
        graph()
            .order()
            .map(|n| Node {
                tool: if n.id.as_str() == "baseline/A~B" {
                    "broken 1".into()
                } else {
                    n.tool.clone()
                },
                ..n.clone()
            })
            .collect(),
    )
    .unwrap();
    let runs = run_graph(
        JobId(7),
        &broken,
        &mut StoreCache(&mut MemoryStore::new()),
        &mut work,
        &mut |e| events.push(e),
        &|| false,
        None,
    );
    assert!(runs.is_none());
    assert_eq!(
        events.last(),
        Some(&JobEvent::Finished {
            job: JobId(7),
            outcome: Outcome::Failed {
                message: "baseline/A~B: no overlap".into()
            }
        })
    );

    // Cancelled after the first step.
    let started = Cell::new(0);
    let mut last = None;
    run_graph(
        JobId(8),
        &graph(),
        &mut StoreCache(&mut MemoryStore::new()),
        &mut work,
        &mut |e| {
            if matches!(e, JobEvent::Progress { .. }) {
                started.set(started.get() + 1);
            }
            last = Some(e);
        },
        &|| started.get() >= 1,
        None,
    );
    assert_eq!(
        last,
        Some(JobEvent::Finished {
            job: JobId(8),
            outcome: Outcome::Cancelled
        })
    );

    let issue = JobEvent::issue(
        JobId(9),
        &Issue::new("occupations/A.toml", "mark PM9 isn't in marks.toml"),
    );
    assert_eq!(
        serde_json::to_string(&issue).unwrap(),
        r#"{"type":"issue-raised","job":9,"file":"occupations/A.toml","message":"mark PM9 isn't in marks.toml"}"#
    );
}
