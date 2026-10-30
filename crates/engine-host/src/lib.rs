//! Runs engine modules natively with wasmtime (ARCHITECTURE.md section 9.3):
//! the same WebAssembly modules the web app runs in its workers, so the CLI
//! and the browser give the same results.
//!
//! - **A fresh instance for every run**, as in the browser, so nothing
//!   carries over from one run to the next.
//! - **One folder**, preopened as the module's working directory: its
//!   inputs are there, and it writes its outputs there. Nothing else on the
//!   machine is visible to it, and it has no network.
//! - **Limits.** A run is stopped after its time limit, and can't grow its
//!   memory past the limit. Its standard output and error are kept in
//!   memory, up to a limit each.
//! - **A trap is an error, not a crash**: the run reports it, with what the
//!   module wrote to standard error.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use wasmtime::{
    Config, Engine, Linker, Module, ResourceLimiter, Store, StoreLimits, StoreLimitsBuilder,
};
use wasmtime_wasi::p1::{WasiP1Ctx, add_to_linker_sync};
use wasmtime_wasi::p2::pipe::MemoryOutputPipe;
use wasmtime_wasi::{FsPerms, I32Exit, WasiCtxBuilder};

/// Why a run couldn't be made or didn't finish.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// The module isn't valid WebAssembly, or not a WASI command.
    #[error("the engine module doesn't load: {0}")]
    Module(String),
    /// The working folder couldn't be given to the module.
    #[error("the run's folder: {0}")]
    Folder(String),
    /// The run went past its time limit.
    #[error("the engine ran for more than {0:?} and was stopped")]
    TimedOut(Duration),
    /// The module trapped: a bug in it, or it ran out of memory.
    #[error("the engine stopped with an error: {message}")]
    Trapped {
        /// What wasmtime said.
        message: String,
        /// What the module wrote to standard error before it stopped.
        stderr: String,
    },
}

/// How a run is limited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How long it may run.
    pub time: Duration,
    /// How much memory it may use, in bytes. WebAssembly's 32-bit limit is
    /// 4 GiB.
    pub memory: usize,
    /// How much standard output and error it may write, each, in bytes.
    pub output: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            // A 24-hour 30 s baseline takes about a minute (M0 spike).
            time: Duration::from_secs(20 * 60),
            memory: 2 << 30,
            output: 64 << 20,
        }
    }
}

/// What a run gave back. Files it wrote are in its folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Its exit status: 0 for success.
    pub status: i32,
    /// What it wrote to standard output.
    pub stdout: Vec<u8>,
    /// What it wrote to standard error.
    pub stderr: Vec<u8>,
}

/// An engine module, compiled once and run any number of times.
pub struct Host {
    engine: Engine,
    module: Module,
}

struct State {
    wasi: WasiP1Ctx,
    limits: StoreLimits,
}

impl Host {
    /// Compiles a module (`rnx2rtkp.wasm`, `convbin.wasm`). The caller
    /// checks its hash first, against the one it was built to.
    pub fn new(wasm: &[u8]) -> Result<Self, HostError> {
        let mut config = Config::new();
        // Time limits interrupt a run at the next epoch check.
        config.epoch_interruption(true);
        let engine = Engine::new(&config).map_err(|e| HostError::Module(e.to_string()))?;
        let module = Module::new(&engine, wasm).map_err(|e| HostError::Module(e.to_string()))?;
        Ok(Self { engine, module })
    }

    /// Runs the module with arguments (the first is the program's name, as
    /// in C's `argv`), in a folder it sees as its working directory.
    pub fn run(&self, args: &[&str], folder: &Path, limits: Limits) -> Result<Output, HostError> {
        let stdout = MemoryOutputPipe::new(limits.output);
        let stderr = MemoryOutputPipe::new(limits.output);
        let mut wasi = WasiCtxBuilder::new();
        wasi.args(args)
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            .preopened_dir(folder, ".", FsPerms::ReadWrite)
            .map_err(|e| HostError::Folder(format!("{}: {e}", folder.display())))?;
        let state = State {
            wasi: wasi.build_p1(),
            limits: StoreLimitsBuilder::new()
                .memory_size(limits.memory)
                .instances(1)
                .build(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits as &mut dyn ResourceLimiter);
        store.set_epoch_deadline(1);

        // The clock that stops a run: it ends the epoch at the time limit,
        // or when the run finishes first.
        let done = Arc::new(AtomicBool::new(false));
        let watchdog = {
            let (engine, done, time) = (self.engine.clone(), done.clone(), limits.time);
            std::thread::spawn(move || {
                let step = Duration::from_millis(50);
                let mut waited = Duration::ZERO;
                while waited < time && !done.load(Ordering::Relaxed) {
                    std::thread::sleep(step);
                    waited += step;
                }
                if !done.load(Ordering::Relaxed) {
                    engine.increment_epoch();
                }
            })
        };

        let mut linker: Linker<State> = Linker::new(&self.engine);
        let result = add_to_linker_sync(&mut linker, |s: &mut State| &mut s.wasi)
            .and_then(|()| linker.instantiate(&mut store, &self.module))
            .and_then(|instance| instance.get_typed_func::<(), ()>(&mut store, "_start"))
            .and_then(|start| start.call(&mut store, ()));
        done.store(true, Ordering::Relaxed);
        let _ = watchdog.join();

        let stderr_text = || String::from_utf8_lossy(&stderr.contents()).into_owned();
        let status = match result {
            Ok(()) => 0,
            Err(e) => match e.downcast_ref::<I32Exit>() {
                Some(exit) => exit.0,
                None if matches!(
                    e.downcast_ref::<wasmtime::Trap>(),
                    Some(wasmtime::Trap::Interrupt)
                ) =>
                {
                    return Err(HostError::TimedOut(limits.time));
                }
                None => {
                    return Err(HostError::Trapped {
                        message: format!("{e:#}"),
                        stderr: stderr_text(),
                    });
                }
            },
        };
        Ok(Output {
            status,
            stdout: stdout.contents().to_vec(),
            stderr: stderr.contents().to_vec(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A WASI command that writes `text` to a file and to standard output,
    /// then exits with `status`.
    fn command(text: &str, status: i32) -> Vec<u8> {
        let len = text.len();
        wat::parse_str(format!(
            r#"(module
  (import "wasi_snapshot_preview1" "fd_write" (func $write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "path_open" (func $open (param i32 i32 i32 i32 i32 i64 i64 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
  (memory (export "memory") 1)
  (data (i32.const 0) "out.txt")
  (data (i32.const 16) "{text}")
  (func (export "_start")
    ;; An iovec at 64: the text.
    (i32.store (i32.const 64) (i32.const 16))
    (i32.store (i32.const 68) (i32.const {len}))
    (drop (call $write (i32.const 1) (i32.const 64) (i32.const 1) (i32.const 72)))
    ;; out.txt in the preopened folder (fd 3), created, writable.
    (drop (call $open (i32.const 3) (i32.const 0) (i32.const 0) (i32.const 7)
      (i32.const 1) (i64.const 64) (i64.const 0) (i32.const 0) (i32.const 80)))
    (drop (call $write (i32.load (i32.const 80)) (i32.const 64) (i32.const 1) (i32.const 72)))
    (call $exit (i32.const {status}))))"#
        ))
        .unwrap()
    }

    fn limits() -> Limits {
        Limits {
            time: Duration::from_secs(10),
            memory: 16 << 20,
            output: 1 << 20,
        }
    }

    #[test]
    fn a_run_writes_its_output_and_files_in_its_folder() {
        let host = Host::new(&command("hello", 3)).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = host.run(&["prog"], dir.path(), limits()).unwrap();
        assert_eq!(out.status, 3);
        assert_eq!(out.stdout, b"hello");
        assert_eq!(std::fs::read(dir.path().join("out.txt")).unwrap(), b"hello");
        // A fresh instance each time.
        let again = host.run(&["prog"], dir.path(), limits()).unwrap();
        assert_eq!(again.stdout, b"hello");
    }

    #[test]
    fn runs_are_stopped_at_their_limits() {
        let dir = tempfile::tempdir().unwrap();
        let forever = wat::parse_str(
            r#"(module (memory (export "memory") 1) (func (export "_start") (loop (br 0))))"#,
        )
        .unwrap();
        let quick = Limits {
            time: Duration::from_millis(200),
            ..limits()
        };
        match Host::new(&forever).unwrap().run(&["p"], dir.path(), quick) {
            Err(HostError::TimedOut(_)) => {}
            other => panic!("{other:?}"),
        }
        // Growing memory past the limit fails in the module (-1), which
        // traps here.
        let greedy = wat::parse_str(
            r#"(module (memory (export "memory") 1)
  (func (export "_start") (if (i32.eq (memory.grow (i32.const 1024)) (i32.const -1)) (then unreachable))))"#,
        )
        .unwrap();
        match Host::new(&greedy)
            .unwrap()
            .run(&["p"], dir.path(), limits())
        {
            Err(HostError::Trapped { message, .. }) => {
                assert!(message.contains("unreachable"), "{message}")
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(Host::new(b"not wasm"), Err(HostError::Module(_))));
        let missing = dir.path().join("missing");
        assert!(matches!(
            Host::new(&command("x", 0))
                .unwrap()
                .run(&["p"], &missing, limits()),
            Err(HostError::Folder(_))
        ));
    }
}
