use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::{Key, KeyBuilder};

/// A node's id, such as `qc/2026-10-14-PM12345-A` or `baseline/A~B`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(String);

impl NodeId {
    /// A node id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The id's text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a node reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// Data from outside the graph, such as a raw file or an occupation's
    /// file, by name and content hash.
    Data {
        /// What it is, such as `raw/PM12345A.rnx`.
        name: String,
        /// The hash of its bytes.
        hash: Key,
    },
    /// Another node's output.
    Node(NodeId),
}

/// One step of processing: what it is, the tool that does it, its settings
/// and its inputs. Its key is the hash of all of these, with its upstream
/// nodes' keys in place of their outputs (ARCHITECTURE.md section 7), so a
/// node whose key is unchanged needn't run again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// The node's id.
    pub id: NodeId,
    /// The tool and its version, such as `qc 0.2.0 web`. A new version gives
    /// new keys, so nothing is reused across tools.
    pub tool: String,
    /// The node's settings, in a canonical form (the caller's choice, such as
    /// sorted TOML).
    pub settings: Vec<u8>,
    /// The inputs, in the order the tool receives them.
    pub inputs: Vec<Input>,
}

/// Why a graph is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GraphError {
    /// Two nodes share an id.
    #[error("node {0} is defined twice")]
    Duplicate(NodeId),
    /// A node reads a node that isn't in the graph.
    #[error("node {node} reads {missing}, which isn't in the graph")]
    Missing {
        /// The node.
        node: NodeId,
        /// What it reads.
        missing: NodeId,
    },
    /// Nodes depend on each other in a cycle.
    #[error("nodes depend on each other in a cycle: {}", .0.iter().map(NodeId::as_str).collect::<Vec<_>>().join(", "))]
    Cycle(Vec<NodeId>),
}

/// A checked graph of nodes, with each node's key.
#[derive(Debug, Clone)]
pub struct Graph {
    nodes: BTreeMap<NodeId, Node>,
    order: Vec<NodeId>,
    keys: BTreeMap<NodeId, Key>,
}

impl Graph {
    /// Checks the nodes form an acyclic graph, orders them so each comes
    /// after its inputs (ties broken by id, so the order is always the same)
    /// and works out every node's key.
    pub fn new(nodes: Vec<Node>) -> Result<Self, GraphError> {
        let mut by_id = BTreeMap::new();
        for node in nodes {
            if by_id.contains_key(&node.id) {
                return Err(GraphError::Duplicate(node.id));
            }
            by_id.insert(node.id.clone(), node);
        }
        let mut waiting: BTreeMap<&NodeId, usize> = BTreeMap::new();
        let mut readers: BTreeMap<&NodeId, Vec<&NodeId>> = BTreeMap::new();
        for node in by_id.values() {
            let mut count = 0;
            for input in &node.inputs {
                if let Input::Node(upstream) = input {
                    if !by_id.contains_key(upstream) {
                        return Err(GraphError::Missing {
                            node: node.id.clone(),
                            missing: upstream.clone(),
                        });
                    }
                    readers.entry(upstream).or_default().push(&node.id);
                    count += 1;
                }
            }
            waiting.insert(&node.id, count);
        }
        // Kahn's algorithm, taking the smallest ready id each time.
        let mut ready: BTreeSet<&NodeId> = waiting
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut order = Vec::with_capacity(by_id.len());
        while let Some(id) = ready.pop_first() {
            order.push(id.clone());
            for reader in readers.get(id).into_iter().flatten() {
                if let Some(n) = waiting.get_mut(reader) {
                    *n -= 1;
                    if *n == 0 {
                        ready.insert(reader);
                    }
                }
            }
        }
        if order.len() < by_id.len() {
            let done: BTreeSet<&NodeId> = order.iter().collect();
            let stuck = by_id
                .keys()
                .filter(|id| !done.contains(id))
                .cloned()
                .collect();
            return Err(GraphError::Cycle(stuck));
        }
        let mut keys = BTreeMap::new();
        for id in &order {
            let node = &by_id[id];
            let mut builder = KeyBuilder::new("node")
                .text("id", id.as_str())
                .text("tool", &node.tool)
                .bytes("settings", &node.settings);
            for input in &node.inputs {
                builder = match input {
                    Input::Data { name, hash } => builder.text("data", name).key("hash", hash),
                    Input::Node(upstream) => builder.key("node", &keys[upstream]),
                };
            }
            keys.insert(id.clone(), builder.finish());
        }
        Ok(Self {
            nodes: by_id,
            order,
            keys,
        })
    }

    /// The nodes, each after its inputs.
    pub fn order(&self) -> impl Iterator<Item = &Node> {
        self.order.iter().map(|id| &self.nodes[id])
    }

    /// A node by id.
    pub fn node(&self, id: &NodeId) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// A node's key.
    pub fn key(&self, id: &NodeId) -> Option<Key> {
        self.keys.get(id).copied()
    }

    /// How many nodes there are.
    pub fn len(&self) -> usize {
        self.order.len()
    }

    /// Whether there are no nodes.
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    use super::{Graph, GraphError, Input, Node, NodeId};
    use crate::Key;

    fn node(id: &str, inputs: &[&str]) -> Node {
        Node {
            id: NodeId::new(id),
            tool: "test 1".into(),
            settings: Vec::new(),
            inputs: inputs
                .iter()
                .map(|i| Input::Node(NodeId::new(*i)))
                .collect(),
        }
    }

    #[test]
    fn nodes_come_after_their_inputs_in_a_stable_order() {
        let graph = Graph::new(vec![
            node("d", &["b", "c"]),
            node("c", &["a"]),
            node("b", &["a"]),
            node("a", &[]),
            node("e", &[]),
        ])
        .unwrap();
        let order: Vec<&str> = graph.order().map(|n| n.id.as_str()).collect();
        assert_eq!(order, ["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn bad_graphs_are_refused() {
        assert_eq!(
            Graph::new(vec![node("a", &[]), node("a", &[])]).unwrap_err(),
            GraphError::Duplicate(NodeId::new("a"))
        );
        assert_eq!(
            Graph::new(vec![node("a", &["z"])]).unwrap_err().to_string(),
            "node a reads z, which isn't in the graph"
        );
        assert_eq!(
            Graph::new(vec![node("a", &[]), node("b", &["c"]), node("c", &["b"])])
                .unwrap_err()
                .to_string(),
            "nodes depend on each other in a cycle: b, c"
        );
    }

    #[test]
    fn keys_change_with_anything_upstream() {
        let data = |bytes: &[u8]| Input::Data {
            name: "raw/a".into(),
            hash: Key::of(bytes),
        };
        let build = |raw: &[u8], tool: &str| {
            let mut a = node("a", &[]);
            a.inputs.push(data(raw));
            let mut b = node("b", &["a"]);
            b.tool = tool.into();
            Graph::new(vec![a, b, node("c", &[])]).unwrap()
        };
        let base = build(b"x", "test 1");
        let key = |g: &Graph, id: &str| g.key(&NodeId::new(id)).unwrap();
        // A change to a raw file changes its node and everything downstream.
        let changed = build(b"y", "test 1");
        assert_ne!(key(&base, "a"), key(&changed, "a"));
        assert_ne!(key(&base, "b"), key(&changed, "b"));
        assert_eq!(key(&base, "c"), key(&changed, "c"));
        // A new tool version changes only the nodes it runs.
        let retooled = build(b"x", "test 2");
        assert_eq!(key(&base, "a"), key(&retooled, "a"));
        assert_ne!(key(&base, "b"), key(&retooled, "b"));
    }
}
