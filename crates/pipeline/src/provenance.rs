//! `provenance.jsonl`: an append-only record of every node of every run
//! (ARCHITECTURE.md section 7), so "how did you get this number?" always has
//! an answer: which inputs, settings, tool and build made each output.

use gnss_store::{ProjectPath, Store, StoreError};
use serde::{Deserialize, Serialize};

use crate::{Graph, Input, Key, NodeRun, Status};

/// The provenance file's path in the project folder.
pub const FILE: &str = "provenance.jsonl";

/// One node of one run: one line of `provenance.jsonl`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    /// When the run started, in UTC, such as `2026-10-15T03:12:44Z`. Every
    /// node of a run has the same value.
    pub run: String,
    /// The app's version.
    pub app: String,
    /// The build, `web` or `native`: results are bit-identical only within one.
    pub build: String,
    /// The node.
    pub node: String,
    /// `computed` or `cached`.
    pub status: String,
    /// The node's key.
    pub key: String,
    /// The tool and its version.
    pub tool: String,
    /// The hash of the node's settings.
    pub settings: String,
    /// Its inputs, as `name@hash` for outside data and `node@key` for other
    /// nodes, in order.
    pub inputs: Vec<String>,
    /// The hash of its output.
    pub output: String,
    /// How long it took in milliseconds, if the host timed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
}

/// What a run records about itself.
#[derive(Debug, Clone, Copy)]
pub struct RunInfo<'a> {
    /// When it started, in UTC.
    pub started: &'a str,
    /// The app's version.
    pub app: &'a str,
    /// `web` or `native`.
    pub build: &'a str,
}

/// The records for a run's nodes, in the order they ran.
pub fn records(info: RunInfo<'_>, graph: &Graph, runs: &[NodeRun]) -> Vec<Record> {
    runs.iter()
        .filter_map(|run| {
            let node = graph.node(&run.id)?;
            let inputs = node
                .inputs
                .iter()
                .map(|input| match input {
                    Input::Data { name, hash } => format!("{name}@{hash}"),
                    Input::Node(id) => {
                        format!(
                            "{id}@{}",
                            graph.key(id).map(|k| k.hex()).unwrap_or_default()
                        )
                    }
                })
                .collect();
            Some(Record {
                run: info.started.to_owned(),
                app: info.app.to_owned(),
                build: info.build.to_owned(),
                node: run.id.to_string(),
                status: match run.status {
                    Status::Computed => "computed",
                    Status::Cached => "cached",
                }
                .to_owned(),
                key: run.key.hex(),
                tool: node.tool.clone(),
                settings: Key::of(&node.settings).hex(),
                inputs,
                output: run.output.hex(),
                ms: run.ms,
            })
        })
        .collect()
}

/// Why `provenance.jsonl` couldn't be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProvenanceError {
    /// The store failed.
    #[error("{0}")]
    Store(#[from] StoreError),
    /// A line isn't a record.
    #[error("provenance.jsonl, line {line}: {message}")]
    BadLine {
        /// The line, from 1.
        line: usize,
        /// What's wrong.
        message: String,
    },
}

/// Adds records to the end of `provenance.jsonl`. Nothing in it is ever
/// changed or removed.
pub fn append<S: Store + ?Sized>(store: &mut S, records: &[Record]) -> Result<(), StoreError> {
    if records.is_empty() {
        return Ok(());
    }
    let path = ProjectPath::new(FILE)?;
    let mut bytes = store.read(&path)?.unwrap_or_default();
    if bytes.last().is_some_and(|b| *b != b'\n') {
        bytes.push(b'\n');
    }
    for record in records {
        // A record is strings, numbers and lists of strings, which always
        // serialise.
        if let Ok(line) = serde_json::to_string(record) {
            bytes.extend_from_slice(line.as_bytes());
            bytes.push(b'\n');
        }
    }
    store.write(&path, &bytes)
}

/// Every record in `provenance.jsonl`, oldest first; none if there's no file.
pub fn read<S: Store + ?Sized>(store: &S) -> Result<Vec<Record>, ProvenanceError> {
    let Some(bytes) = store.read(&ProjectPath::new(FILE)?)? else {
        return Ok(Vec::new());
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line).map_err(|e| ProvenanceError::BadLine {
                line: index + 1,
                message: e.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use gnss_store::{MemoryStore, ProjectPath, Store};

    use super::{FILE, ProvenanceError, read};

    #[test]
    fn a_damaged_line_is_reported_with_its_number() {
        let mut store = MemoryStore::new();
        assert!(read(&store).unwrap().is_empty());
        store
            .write(&ProjectPath::new(FILE).unwrap(), b"\n{\"run\":\"x\"}\n")
            .unwrap();
        match read(&store) {
            Err(ProvenanceError::BadLine { line, message }) => {
                assert_eq!(line, 2);
                assert!(message.contains("missing field"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }
}
