//! Optional relay for data sources that browsers can't reach directly.
//!
//! A small, stateless proxy that forwards GET requests only to hosts declared
//! in the jurisdiction packs, and keeps no logs or copies. It's off by
//! default, because it can see which files are requested. See
//! `docs/ARCHITECTURE.md` section 13. Not implemented yet (M7).

use std::process::ExitCode;

fn main() -> ExitCode {
    eprintln!("gnss-relay isn't implemented yet (milestone M7)");
    ExitCode::FAILURE
}
