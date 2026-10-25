use std::collections::BTreeMap;

use gnss_store::{ProjectPath, Store, StoreError};

use crate::{Graph, Input, Key, Node, NodeId};

/// The work a node does: given the node and its upstream nodes' outputs, in
/// the order of its inputs, its output.
pub type Compute<'a, E> = dyn FnMut(&Node, &[&[u8]]) -> Result<Vec<u8>, E> + 'a;

/// Where node outputs are kept, by key.
pub trait Cache {
    /// The output stored under a key, if any.
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>, StoreError>;
    /// Stores an output under a key.
    fn put(&mut self, key: &Key, bytes: &[u8]) -> Result<(), StoreError>;
}

/// A cache in a project's `.cache/` folder, one file per key:
/// `.cache/<first two hex digits>/<key>`. Everything in it can be made
/// again, so it can be deleted at any time.
pub struct StoreCache<'a, S: Store + ?Sized>(pub &'a mut S);

fn cache_path(key: &Key) -> Result<ProjectPath, StoreError> {
    let hex = key.hex();
    ProjectPath::new(format!(".cache/{}/{hex}", &hex[..2]))
}

impl<S: Store + ?Sized> Cache for StoreCache<'_, S> {
    fn get(&self, key: &Key) -> Result<Option<Vec<u8>>, StoreError> {
        self.0.read(&cache_path(key)?)
    }

    fn put(&mut self, key: &Key, bytes: &[u8]) -> Result<(), StoreError> {
        self.0.write(&cache_path(key)?, bytes)
    }
}

/// Whether a node ran or its output was reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// It ran.
    Computed,
    /// Its key was in the cache, so its output was reused.
    Cached,
}

/// What happened to one node in a run, for progress and provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeRun {
    /// The node.
    pub id: NodeId,
    /// Its key.
    pub key: Key,
    /// Whether it ran.
    pub status: Status,
    /// The hash of its output.
    pub output: Key,
}

/// Progress, reported as a run goes.
#[derive(Debug, Clone, Copy)]
pub enum Event<'a> {
    /// A node is about to be looked up and, if need be, run.
    Started {
        /// The node.
        node: &'a NodeId,
        /// Its position in the run, from 0.
        index: usize,
        /// How many nodes there are.
        total: usize,
    },
    /// A node is done.
    Finished(&'a NodeRun),
}

/// Why a run stopped.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError<E: std::fmt::Display> {
    /// The cache couldn't be read or written.
    #[error("the cache: {0}")]
    Cache(#[from] StoreError),
    /// A node failed.
    #[error("{node}: {error}")]
    Node {
        /// The node.
        node: NodeId,
        /// Why.
        error: E,
    },
    /// The run was cancelled between nodes. Nodes already done stay cached.
    #[error("cancelled")]
    Cancelled,
}

/// How to run: what to tell the host as it goes, and how the host asks it
/// to stop. The host supplies both, as it supplies the executor
/// (ARCHITECTURE.md section 7).
pub struct Hooks<'a> {
    on_event: Box<dyn FnMut(Event<'_>) + 'a>,
    cancelled: Box<dyn Fn() -> bool + 'a>,
}

impl<'a> Hooks<'a> {
    /// Hooks that report each event and check `cancelled` before each node.
    pub fn new(on_event: impl FnMut(Event<'_>) + 'a, cancelled: impl Fn() -> bool + 'a) -> Self {
        Self {
            on_event: Box::new(on_event),
            cancelled: Box::new(cancelled),
        }
    }

    /// No progress reporting, and never cancelled.
    pub fn none() -> Self {
        Self::new(|_| {}, || false)
    }
}

/// The outputs of a node's upstream nodes, in the order of its inputs.
fn upstream<'o>(node: &Node, outputs: &'o BTreeMap<NodeId, Vec<u8>>) -> Vec<&'o [u8]> {
    node.inputs
        .iter()
        .filter_map(|input| match input {
            Input::Node(id) => outputs.get(id).map(Vec::as_slice),
            Input::Data { .. } => None,
        })
        .collect()
}

/// Runs a graph: each node in order, reusing its cached output if its key is
/// in the cache, and otherwise calling `compute` with the node and its
/// upstream nodes' outputs, then caching what it returns.
pub fn run<C, E>(
    graph: &Graph,
    cache: &mut C,
    compute: &mut Compute<'_, E>,
    hooks: &mut Hooks<'_>,
) -> Result<Vec<NodeRun>, RunError<E>>
where
    C: Cache + ?Sized,
    E: std::fmt::Display,
{
    let total = graph.len();
    let mut outputs: BTreeMap<NodeId, Vec<u8>> = BTreeMap::new();
    let mut runs = Vec::with_capacity(total);
    for (index, node) in graph.order().enumerate() {
        if (hooks.cancelled)() {
            return Err(RunError::Cancelled);
        }
        (hooks.on_event)(Event::Started {
            node: &node.id,
            index,
            total,
        });
        let Some(key) = graph.key(&node.id) else {
            continue;
        };
        let (bytes, status) = match cache.get(&key)? {
            Some(bytes) => (bytes, Status::Cached),
            None => {
                let bytes =
                    compute(node, &upstream(node, &outputs)).map_err(|error| RunError::Node {
                        node: node.id.clone(),
                        error,
                    })?;
                cache.put(&key, &bytes)?;
                (bytes, Status::Computed)
            }
        };
        let record = NodeRun {
            id: node.id.clone(),
            key,
            status,
            output: Key::of(&bytes),
        };
        (hooks.on_event)(Event::Finished(&record));
        runs.push(record);
        outputs.insert(node.id.clone(), bytes);
    }
    Ok(runs)
}

/// A node whose output, computed afresh, isn't what the cache holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// The node.
    pub id: NodeId,
    /// The hash of the cached output, or `None` if there was none.
    pub cached: Option<Key>,
    /// The hash of the output computed now.
    pub computed: Key,
}

/// Recomputes every node from scratch, ignoring and not changing the cache,
/// and compares each output with the cached one: the audit answer to "how
/// did you get this number?" (ARCHITECTURE.md section 7). An empty result
/// means everything was reproduced bit for bit.
pub fn verify<C, E>(
    graph: &Graph,
    cache: &C,
    compute: &mut Compute<'_, E>,
    hooks: &mut Hooks<'_>,
) -> Result<Vec<Mismatch>, RunError<E>>
where
    C: Cache + ?Sized,
    E: std::fmt::Display,
{
    let total = graph.len();
    let mut outputs: BTreeMap<NodeId, Vec<u8>> = BTreeMap::new();
    let mut mismatches = Vec::new();
    for (index, node) in graph.order().enumerate() {
        if (hooks.cancelled)() {
            return Err(RunError::Cancelled);
        }
        (hooks.on_event)(Event::Started {
            node: &node.id,
            index,
            total,
        });
        let Some(key) = graph.key(&node.id) else {
            continue;
        };
        let bytes = compute(node, &upstream(node, &outputs)).map_err(|error| RunError::Node {
            node: node.id.clone(),
            error,
        })?;
        let computed = Key::of(&bytes);
        let cached = cache.get(&key)?.map(|b| Key::of(&b));
        if cached != Some(computed) {
            mismatches.push(Mismatch {
                id: node.id.clone(),
                cached,
                computed,
            });
        }
        outputs.insert(node.id.clone(), bytes);
    }
    Ok(mismatches)
}
