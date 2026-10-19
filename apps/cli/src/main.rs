//! Command-line interface. See `docs/ARCHITECTURE.md` section 15.

use std::process::ExitCode;

const USAGE: &str = "\
usage: gnss <command> [options]

Commands arrive milestone by milestone (docs/ROADMAP.md). So far:
  --version    print the version and build
  --help       print this help";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => {
            println!(
                "gnss {} ({})",
                gnss_api::VERSION,
                gnss_api::BuildTarget::current().as_str()
            );
            ExitCode::SUCCESS
        }
        Some("--help" | "-h") | None => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("gnss: '{other}' isn't implemented yet\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}
