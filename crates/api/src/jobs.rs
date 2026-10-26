//! Long-running work as jobs (ARCHITECTURE.md section 8): a job has an id,
//! and reports its progress, the issues it finds, and how it ended, as
//! [`JobEvent`]s. Every front end shows the same events: the web app gets
//! them from the core worker as JSON messages, the CLI prints them.

use std::cell::Cell;

use gnss_model::Issue;
use gnss_pipeline::{Cache, Compute, Event, Graph, Hooks, NodeRun, RunError, Status, run};
use serde::Serialize;

/// A job's id, unique within a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub struct JobId(pub u64);

/// Hands out job ids.
#[derive(Debug, Default)]
pub struct Jobs {
    next: u64,
}

impl Jobs {
    /// A new job's id.
    pub fn start(&mut self) -> JobId {
        self.next += 1;
        JobId(self.next)
    }
}

/// Something a job reports. Serialised with a `type` field, such as
/// `{"type":"progress","job":1,"done":3,"total":7,"step":"baseline/A~B"}`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum JobEvent {
    /// A step has started.
    Progress {
        /// The job.
        job: JobId,
        /// Steps finished so far.
        done: usize,
        /// Steps in all.
        total: usize,
        /// The step starting.
        step: String,
    },
    /// A problem the person should see, with the file it's in. The job may
    /// carry on.
    IssueRaised {
        /// The job.
        job: JobId,
        /// The file, relative to the project folder.
        file: String,
        /// What's wrong, and what to do about it.
        message: String,
    },
    /// The job is over.
    Finished {
        /// The job.
        job: JobId,
        /// How it ended.
        outcome: Outcome,
    },
}

impl JobEvent {
    /// An issue as an event.
    pub fn issue(job: JobId, issue: &Issue) -> Self {
        Self::IssueRaised {
            job,
            file: issue.file.clone(),
            message: issue.message.clone(),
        }
    }
}

/// How a job ended.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum Outcome {
    /// It finished.
    Succeeded {
        /// Steps that ran.
        computed: usize,
        /// Steps whose cached results were reused.
        cached: usize,
    },
    /// A step failed.
    Failed {
        /// Which step, and why.
        message: String,
    },
    /// It was cancelled. Steps that finished stay cached.
    Cancelled,
}

/// Runs a pipeline graph as a job: reports each step as it starts and how
/// the job ended, and stops between steps when `cancelled` says so. Returns
/// the steps' records, for provenance, if it finished.
pub fn run_graph<C, E>(
    job: JobId,
    graph: &Graph,
    cache: &mut C,
    compute: &mut Compute<'_, E>,
    emit: &mut dyn FnMut(JobEvent),
    cancelled: &dyn Fn() -> bool,
    clock: Option<&dyn Fn() -> f64>,
) -> Option<Vec<NodeRun>>
where
    C: Cache + ?Sized,
    E: std::fmt::Display,
{
    let done = Cell::new(0);
    let total = graph.len();
    let result = {
        let mut hooks = Hooks::new(
            |event| match event {
                Event::Started { node, .. } => emit(JobEvent::Progress {
                    job,
                    done: done.get(),
                    total,
                    step: node.to_string(),
                }),
                Event::Finished(_) => done.set(done.get() + 1),
            },
            cancelled,
        );
        if let Some(clock) = clock {
            hooks = hooks.with_clock(clock);
        }
        run(graph, cache, compute, &mut hooks)
    };
    let (outcome, runs) = match result {
        Ok(runs) => {
            let computed = runs.iter().filter(|r| r.status == Status::Computed).count();
            (
                Outcome::Succeeded {
                    computed,
                    cached: runs.len() - computed,
                },
                Some(runs),
            )
        }
        Err(RunError::Cancelled) => (Outcome::Cancelled, None),
        Err(e) => (
            Outcome::Failed {
                message: e.to_string(),
            },
            None,
        ),
    };
    emit(JobEvent::Finished { job, outcome });
    runs
}
