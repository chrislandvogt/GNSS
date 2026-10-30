//! Optional relay for data sources that browsers can't reach directly.
//!
//! A small, stateless proxy that forwards GET requests only to hosts declared
//! in the jurisdiction packs, and keeps no logs or copies. It's off by
//! default, because it can see which files are requested. See
//! `docs/ARCHITECTURE.md` section 13 and the crate's documentation.
//!
//! ```text
//! gnss-relay [--listen 127.0.0.1:8787] [--allow-origin ORIGIN] [--pack PACK.toml]
//! ```

use std::net::TcpListener;
use std::process::ExitCode;
use std::sync::Arc;

use gnss_relay::{Config, Network, serve};

const USAGE: &str =
    "usage: gnss-relay [--listen ADDRESS:PORT] [--allow-origin ORIGIN] [--pack PACK.toml]

  --listen        where to listen (default 127.0.0.1:8787, this machine only)
  --allow-origin  the web app's origin, to serve only it (default *)
  --pack          a jurisdiction pack whose sources' hosts to allow
                  (default the built-in Australian pack)

It forwards GET requests only to the hosts the pack declares, and keeps no
logs or copies.";

fn main() -> ExitCode {
    let mut listen = "127.0.0.1:8787".to_owned();
    let mut allow_origin = "*".to_owned();
    let mut pack = include_str!("../../../packs/au/pack.toml").to_owned();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        let result = match arg.as_str() {
            "--listen" => value().map(|v| listen = v),
            "--allow-origin" => value().map(|v| allow_origin = v),
            "--pack" => value().and_then(|path| {
                std::fs::read_to_string(&path)
                    .map(|text| pack = text)
                    .map_err(|e| format!("{path}: {e}"))
            }),
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => Err(format!("unknown argument {arg}")),
        };
        if let Err(message) = result {
            eprintln!("gnss-relay: {message}\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    }
    let sources = match gnss_fetch::read_sources(&pack) {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            eprintln!("gnss-relay: the pack declares no sources");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("gnss-relay: {e}");
            return ExitCode::FAILURE;
        }
    };
    let listener = match TcpListener::bind(&listen) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("gnss-relay: {listen}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut hosts: Vec<&str> = sources
        .iter()
        .flat_map(|s| s.hosts.iter().map(String::as_str))
        .collect();
    hosts.sort_unstable();
    hosts.dedup();
    // The only thing it ever prints: no requests are logged.
    println!(
        "gnss-relay listening on http://{listen}, for {}",
        hosts.join(", ")
    );
    let config = Arc::new(Config {
        sources,
        allow_origin,
        max_connections: 32,
    });
    serve(listener, config, Arc::new(Network::new()));
    ExitCode::SUCCESS
}
