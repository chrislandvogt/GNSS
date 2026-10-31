//! Command-line interface. See `docs/ARCHITECTURE.md` section 15.
//!
//! A project is a folder: `project.toml` (settings, imports and edits),
//! `raw/` (the imported files, never changed, with their SHA-256 in
//! `raw.lock`) and `results/` (reports and exports, which can always be made
//! again). Every command takes `--project` (the current folder by default)
//! and `--json`.
//!
//! Exit codes: 0 success, 1 error, 2 the adjustment ran but a statistical
//! test failed or there was no redundancy to test with (from `adjust` and
//! `report`), so scripts can stop on it.

mod folder;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use gnss_api::project::{
    self, AdjustmentSettings, EditRecord, Format, Import, Project, Run, Summary,
};
use gnss_datum::{CoordinateEpoch, FrameId};
use serde::Serialize;

use crate::folder::Folder;

/// Exit code for a statistical test that failed.
const TESTS_FAILED: u8 = 2;

#[derive(Parser)]
#[command(
    name = "gnss",
    version = concat!(env!("CARGO_PKG_VERSION"), " (native)"),
    about = "Adjust GNSS networks and report on them: a second opinion on vectors from any \
             processing package.",
    after_help = "Exit codes: 0 success; 1 error; 2 the adjustment ran, but a statistical test \
                  failed or there was no redundancy to test with (adjust and report), or \
                  something needed couldn't be fetched (fetch)."
)]
struct Cli {
    /// The project folder.
    #[arg(long, global = true, default_value = ".")]
    project: PathBuf,
    /// Print the outcome as JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a project folder.
    Init(InitArgs),
    /// Import vectors or control into the project.
    #[command(subcommand)]
    Import(ImportCommand),
    /// Adjust the network and summarise the tests.
    Adjust(AdjustArgs),
    /// Write the HTML report.
    Report(ReportArgs),
    /// Export the results.
    Export(ExportArgs),
    /// Leave an observation out of the adjustment, with a reason.
    Exclude(ExcludeArgs),
    /// Check RINEX observation files: equipment, completeness, gaps, cycle
    /// slips and multipath, and with navigation files, elevation coverage.
    Qc(QcArgs),
    /// Check the occupations' RINEX headers against the occupations (the
    /// field record), as corrected by overlays; with --fix, write overlays
    /// for what an overlay can correct.
    Reconcile(ReconcileArgs),
    /// Write the occupations' RINEX files as plain RINEX, with their
    /// overlays applied, for delivery.
    ExportRinex(ExportRinexArgs),
    /// Fetch what the occupations need from outside the project (CORS
    /// observations, navigation data, precise products and ANTEX) into the
    /// shared download cache, and record it in data.toml.
    Fetch(FetchArgs),
    /// Offline data packs: the files a project fetched, for a machine with
    /// no network.
    #[command(subcommand)]
    Data(DataCommand),
    /// Process baselines with RTKLIB-EX: between overlapping occupations,
    /// and from each occupation to the CORS data fetched for it. Fixed
    /// solutions are recorded for the adjustment.
    Process(ProcessArgs),
    /// Serve the web app on this machine, from a release bundle, with a
    /// private relay for the data sources a browser can't reach. It answers
    /// only this machine's own pages, and works with no network but for
    /// the relay.
    Serve(ServeArgs),
}

#[derive(Args)]
struct ProcessArgs {
    /// The engine module, rnx2rtkp.wasm, checked against the hash it was
    /// built to. Default: $GNSS_ENGINE, or rnx2rtkp.wasm beside this
    /// program.
    #[arg(long, value_name = "WASM")]
    engine: Option<PathBuf>,
    /// The download cache. Default: $GNSS_CACHE, or the user's cache folder.
    #[arg(long, value_name = "DIR")]
    cache: Option<PathBuf>,
}

#[derive(Args)]
struct ServeArgs {
    /// The web app: a release bundle's zip (gnss-web-<version>.zip) or a
    /// folder holding it.
    #[arg(long, value_name = "ZIP or FOLDER")]
    bundle: PathBuf,
    /// The port on 127.0.0.1 to serve on.
    #[arg(long, default_value_t = 8765)]
    port: u16,
}

#[derive(Args)]
struct FetchArgs {
    /// CORS stations to use with every occupation, by four-character id
    /// (comma-separated). Otherwise the nearest public ones to each mark.
    #[arg(long, value_delimiter = ',', value_name = "ID")]
    cors: Vec<String>,
    /// How many of the nearest CORS to use, without --cors.
    #[arg(long, default_value_t = 2, value_name = "N")]
    nearest: usize,
    /// How far away a nearest CORS can be, in km.
    #[arg(long, default_value_t = 100.0, value_name = "KM")]
    within: f64,
    /// Precise orbits, clocks and Earth rotation parameters too, from CODE.
    #[arg(long)]
    precise: bool,
    /// With --precise: CODE's multi-GNSS products, which add BeiDou and
    /// QZSS, where they're out.
    #[arg(long)]
    multi_gnss: bool,
    /// Leave out the ANTEX file (igs20.atx, as the IGS's weekly release the pack pins).
    #[arg(long)]
    no_antex: bool,
    /// Minutes of CORS data either side of each occupation.
    #[arg(long, default_value_t = 5, value_name = "MINUTES")]
    pad: i64,
    /// The download cache. Default: $GNSS_CACHE, or the user's cache folder.
    #[arg(long, value_name = "DIR")]
    cache: Option<PathBuf>,
    /// Only list what's needed; fetch nothing.
    #[arg(long)]
    dry_run: bool,
    /// A relay (gnss-relay) to reach the sources a browser can't, such as
    /// http://127.0.0.1:8787. Not needed here, where every source is
    /// reached directly; for checking a relay.
    #[arg(long, value_name = "URL")]
    relay: Option<String>,
}

#[derive(Args)]
struct ReconcileArgs {
    /// Write overlays that make the headers agree with the occupations.
    #[arg(long, requires = "by")]
    fix: bool,
    /// Who's making the corrections, for the overlays.
    #[arg(long)]
    by: Option<String>,
}

#[derive(Args)]
struct ExportRinexArgs {
    /// The folder to write to (default results/rinex/).
    #[arg(long)]
    output: Option<PathBuf>,
}

#[derive(Args)]
struct QcArgs {
    /// The files, plain or Hatanaka-compressed RINEX 2, 3 or 4.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// Navigation files (RINEX 2, 3 or 4) for elevation coverage: how much
    /// of what was above the cutoff was tracked.
    #[arg(long, value_name = "FILE")]
    nav: Vec<PathBuf>,
}

#[derive(Args)]
struct InitArgs {
    /// The folder to create.
    dir: PathBuf,
    /// The project's name, for the report.
    #[arg(long)]
    name: Option<String>,
    /// The frame to adjust in.
    #[arg(long, default_value = "GDA2020")]
    frame: String,
    /// The coordinate epoch, for a dynamic frame, as a decimal year.
    #[arg(long)]
    epoch: Option<f64>,
}

#[derive(Subcommand)]
enum ImportCommand {
    /// GNSS vectors: DynaML, STAR*NET, TDEF, RTKLIB or CSV.
    Vectors(ImportArgs<VectorFormat>),
    /// Control: DynaML stations, CSV or SINEX.
    Control(ImportArgs<ControlFormat>),
    /// RINEX observation files: each set-up found in them becomes an
    /// occupation, and marks not yet in marks.toml are added.
    Rinex(RinexArgs),
}

#[derive(Args)]
struct RinexArgs {
    /// The files, plain or Hatanaka-compressed RINEX 2, 3 or 4.
    #[arg(required = true)]
    files: Vec<PathBuf>,
    /// The mark every file was on, instead of their headers' marker names.
    #[arg(long)]
    mark: Option<String>,
    /// Files of one set-up are one occupation if the next starts within
    /// this many seconds of the last ending.
    #[arg(long, value_name = "SECONDS", default_value_t = 300.0)]
    join: f64,
}

#[derive(Args)]
struct ImportArgs<F: ValueEnum + Clone + Send + Sync + 'static> {
    /// The file.
    file: PathBuf,
    /// Its format. Without one, it's guessed from the file's contents and
    /// name, and the guess is shown.
    #[arg(long)]
    format: Option<F>,
    /// The frame of its contents, for formats that don't say (STAR*NET, TDEF,
    /// CSV without a frame column, RTKLIB, SINEX without one in its comments).
    #[arg(long)]
    frame: Option<String>,
    /// The epoch of its contents, as a decimal year or date, for a dynamic frame.
    #[arg(long)]
    epoch: Option<String>,
    /// For an RTKLIB solution: the base's mark name.
    #[arg(long)]
    base: Option<String>,
    /// For an RTKLIB solution: the rover's mark name.
    #[arg(long)]
    rover: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
enum VectorFormat {
    Dynaml,
    Starnet,
    Tdef,
    Rtklib,
    Csv,
}

#[derive(Clone, Copy, ValueEnum)]
enum ControlFormat {
    Dynaml,
    Csv,
    Sinex,
}

#[derive(Args, Default)]
struct AdjustArgs {
    /// Minimally constrained, by inner constraints, for this run.
    #[arg(long, conflicts_with = "constrained")]
    minimal: bool,
    /// Constrained by the control, for this run.
    #[arg(long)]
    constrained: bool,
}

#[derive(Args)]
struct ReportArgs {
    /// Where to write it (default results/report.html).
    #[arg(long)]
    output: Option<PathBuf>,
    #[command(flatten)]
    datum: AdjustArgs,
}

#[derive(Clone, Copy, ValueEnum)]
enum ExportFormat {
    Dynaml,
    Starnet,
    Landxml,
    Gpkg,
    Csv,
}

#[derive(Args)]
struct ExportArgs {
    /// The format.
    #[arg(long)]
    format: ExportFormat,
    /// Where to write it: a file, or a folder for formats that write two
    /// (default results/).
    #[arg(long)]
    output: Option<PathBuf>,
    /// How many times their true size to draw error ellipses in a GeoPackage.
    #[arg(long, default_value_t = 1000.0)]
    ellipse_scale: f64,
    #[command(flatten)]
    datum: AdjustArgs,
}

#[derive(Args)]
struct ExcludeArgs {
    /// The observation's identifier, as the report shows it (file:line).
    observation: String,
    /// Why it's left out.
    #[arg(long)]
    reason: String,
}

/// A failure, with the exit code it gives.
struct Failure {
    message: String,
    code: u8,
}

impl<E: std::fmt::Display> From<E> for Failure {
    fn from(e: E) -> Self {
        Failure {
            message: e.to_string(),
            code: 1,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match dispatch(&cli) {
        Ok(code) => ExitCode::from(code),
        Err(failure) => {
            if cli.json {
                println!("{}", serde_json::json!({ "error": failure.message }));
            } else {
                eprintln!("gnss: {}", failure.message);
            }
            ExitCode::from(failure.code)
        }
    }
}

fn dispatch(cli: &Cli) -> Result<u8, Failure> {
    match &cli.command {
        Command::Init(args) => init(cli, args),
        Command::Import(ImportCommand::Vectors(args)) => {
            let format = args.format.map(|f| match f {
                VectorFormat::Dynaml => Format::Dynaml,
                VectorFormat::Starnet => Format::Starnet,
                VectorFormat::Tdef => Format::Tdef,
                VectorFormat::Rtklib => Format::Rtklib,
                VectorFormat::Csv => Format::CsvVectors,
            });
            let allowed = [
                Format::Dynaml,
                Format::Starnet,
                Format::Tdef,
                Format::Rtklib,
                Format::CsvVectors,
            ];
            import(cli, format, &allowed, args)
        }
        Command::Import(ImportCommand::Control(args)) => {
            let format = args.format.map(|f| match f {
                ControlFormat::Dynaml => Format::Dynaml,
                ControlFormat::Csv => Format::CsvControl,
                ControlFormat::Sinex => Format::Sinex,
            });
            let allowed = [Format::Dynaml, Format::CsvControl, Format::Sinex];
            import(cli, format, &allowed, args)
        }
        Command::Import(ImportCommand::Rinex(args)) => import_rinex(cli, args),
        Command::Adjust(args) => adjust(cli, args),
        Command::Report(args) => report(cli, args),
        Command::Export(args) => export_results(cli, args),
        Command::Exclude(args) => exclude(cli, args),
        Command::Qc(args) => qc(cli, args),
        Command::Reconcile(args) => reconcile(cli, args),
        Command::ExportRinex(args) => export_rinex(cli, args),
        Command::Fetch(args) => fetch(cli, args),
        Command::Data(command) => data(cli, command),
        Command::Process(args) => process(cli, args),
        Command::Serve(args) => serve(args),
    }
}

#[derive(Subcommand)]
enum DataCommand {
    /// Write the files data.toml records, from the download cache, to an
    /// offline data pack (a zip with a manifest), for a machine with no
    /// network.
    Export(DataExportArgs),
    /// Read an offline data pack into the download cache, checking every
    /// file against its hash first.
    Import(DataImportArgs),
}

#[derive(Args)]
struct DataExportArgs {
    /// The pack to write.
    output: PathBuf,
    /// The download cache. Default: $GNSS_CACHE, or the user's cache folder.
    #[arg(long, value_name = "DIR")]
    cache: Option<PathBuf>,
}

#[derive(Args)]
struct DataImportArgs {
    /// The pack to read.
    pack: PathBuf,
    /// The download cache. Default: $GNSS_CACHE, or the user's cache folder.
    #[arg(long, value_name = "DIR")]
    cache: Option<PathBuf>,
}

/// The engine modules' hashes, as their reproducible build gives them.
const ENGINE_SHA256: &str = include_str!("../../../engine-modules/rtklib-ex/golden.sha256");

/// Runs rnx2rtkp natively, a fresh instance for each run, in a folder made
/// for it and removed after.
struct NativeEngine {
    host: gnss_engine_host::Host,
}

impl gnss_api::process::Engine for NativeEngine {
    fn run(
        &mut self,
        inputs: &[(String, Vec<u8>)],
        args: &[String],
        output: &str,
    ) -> Result<gnss_api::process::EngineRun, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static RUN: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "gnss-run-{}-{}",
            std::process::id(),
            RUN.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let result = (|| {
            for (name, bytes) in inputs {
                std::fs::write(dir.join(name), bytes).map_err(|e| e.to_string())?;
            }
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let out = self
                .host
                .run(&args, &dir, gnss_engine_host::Limits::default())
                .map_err(|e| e.to_string())?;
            Ok(gnss_api::process::EngineRun {
                status: out.status,
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
                output: std::fs::read(dir.join(output)).ok(),
            })
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
}

fn process(cli: &Cli, args: &ProcessArgs) -> Result<u8, Failure> {
    use gnss_api::fetch::gnss_fetch::Cache;
    let folder = Folder::open(&cli.project)?;
    let mut store = folder.store();
    let path = match (&args.engine, std::env::var_os("GNSS_ENGINE")) {
        (Some(p), _) => p.clone(),
        (None, Some(p)) => PathBuf::from(p),
        (None, None) => std::env::current_exe()?
            .parent()
            .map(|d| d.join("rnx2rtkp.wasm"))
            .ok_or("can't find the engine: give --engine")?,
    };
    let wasm = std::fs::read(&path).map_err(|e| {
        format!(
            "the engine {}: {e}; give --engine or set GNSS_ENGINE",
            path.display()
        )
    })?;
    let expected = ENGINE_SHA256
        .lines()
        .find_map(|l| l.strip_suffix("  rnx2rtkp.wasm"))
        .ok_or("no recorded hash for rnx2rtkp.wasm")?;
    if gnss_store::sha256_hex(&wasm) != expected {
        return Err(format!(
            "{} isn't the engine this version was built with (its SHA-256 differs)",
            path.display()
        )
        .into());
    }
    let mut engine = NativeEngine {
        host: gnss_engine_host::Host::new(&wasm)?,
    };
    let cache = Cache::new(gnss_store::FsStore::new(cache_dir(args.cache.as_ref())?));
    let report = gnss_api::process::process(
        &mut store,
        &cache,
        &mut engine,
        gnss_api::process::MIN_OVERLAP,
    )?;
    let used = report.baselines.iter().filter(|b| b.used).count();
    print(cli, &report, || {
        for b in &report.baselines {
            let what = match (&b.quality, b.length) {
                (Some(q), Some(l)) => format!(
                    "{q}, {} satellites, ratio {:.1}, {:.3} km",
                    b.satellites.unwrap_or(0),
                    b.ratio.unwrap_or(0.0),
                    l / 1000.0
                ),
                (Some(q), None) => q.clone(),
                _ => "not processed".into(),
            };
            println!(
                "{} to {} ({} to {}): {what}",
                b.base, b.rover, b.start, b.end
            );
            if let Some(p) = &b.problem {
                println!("  {p}");
            }
        }
        for n in &report.notes {
            println!("note: {n}");
        }
        println!(
            "{used} of {} baselines fixed and recorded for the adjustment.",
            report.baselines.len()
        );
    })?;
    Ok(if used == report.baselines.len() { 0 } else { 2 })
}

fn serve(args: &ServeArgs) -> Result<u8, Failure> {
    use std::sync::Arc;

    use gnss_relay::{Config, Network, Site};
    // This machine only: browsers treat it as a secure context, and nothing
    // else on the network can reach it.
    let listener = std::net::TcpListener::bind(("127.0.0.1", args.port))
        .map_err(|e| format!("127.0.0.1:{}: {e}", args.port))?;
    let port = listener.local_addr()?.port();
    let bundle = &args.bundle;
    let site = if bundle.is_dir() {
        Site::from_dir(bundle, port)
    } else {
        std::fs::read(bundle)
            .map_err(|e| e.to_string())
            .and_then(|zip| Site::from_zip(&zip, port))
    }
    .map_err(|e| format!("{}: {e}", bundle.display()))?;
    let sources = gnss_api::fetch::gnss_fetch::read_sources(gnss_api::fetch::AU_PACK)?;
    let origin = format!("http://127.0.0.1:{port}");
    // The only thing it prints: requests aren't logged.
    println!(
        "Serving the web app ({} files) at {origin}/ with its relay at {origin}{}/; Ctrl+C stops it.",
        site.len(),
        gnss_relay::site::RELAY
    );
    let config = Arc::new(Config {
        sources,
        allow_origin: origin,
        max_connections: 32,
        site: Some(site),
    });
    gnss_relay::serve(listener, config, Arc::new(Network::new()));
    Ok(0)
}

fn data(cli: &Cli, command: &DataCommand) -> Result<u8, Failure> {
    use gnss_api::fetch::gnss_fetch::{Cache, pack};
    match command {
        DataCommand::Export(args) => {
            let folder = Folder::open(&cli.project)?;
            let text = std::fs::read_to_string(folder.root().join("data.toml"))
                .map_err(|_| "there's no data.toml: run gnss fetch first")?;
            let data =
                gnss_model::DataFiles::from_toml(&text).map_err(|e| format!("data.toml: {e}"))?;
            let cache_dir = cache_dir(args.cache.as_ref())?;
            let cache = Cache::new(gnss_store::FsStore::new(&cache_dir));
            let mut entries = Vec::new();
            for f in &data.files {
                let entry = cache
                    .get(&f.source, &f.name)
                    .map_err(|e| e.to_string())?
                    .filter(|e| e.sha256 == f.sha256)
                    .ok_or_else(|| {
                        format!(
                            "{} isn't in the cache as data.toml records it; run gnss fetch again",
                            f.name
                        )
                    })?;
                if !entries.contains(&entry) {
                    entries.push(entry);
                }
            }
            let (stamp, _, _) = now();
            let created = format!("{}Z", stamp.get(..19).unwrap_or(&stamp));
            let zip = pack::export(&cache, &entries, &data.attributions, &created)?;
            std::fs::write(&args.output, &zip)
                .map_err(|e| format!("{}: {e}", args.output.display()))?;
            let bytes: u64 = entries.iter().map(|e| e.bytes).sum();
            print(
                cli,
                &serde_json::json!({ "pack": args.output.display().to_string(), "files": entries.len(), "bytes": bytes }),
                || {
                    println!(
                        "Wrote {}: {} files, {:.1} MB.",
                        args.output.display(),
                        entries.len(),
                        bytes as f64 / 1e6
                    );
                },
            )?;
            Ok(0)
        }
        DataCommand::Import(args) => {
            let zip =
                std::fs::read(&args.pack).map_err(|e| format!("{}: {e}", args.pack.display()))?;
            let cache_dir = cache_dir(args.cache.as_ref())?;
            let mut cache = Cache::new(gnss_store::FsStore::new(&cache_dir));
            let manifest = pack::import(&mut cache, &zip)?;
            let names: Vec<&str> = manifest.files.iter().map(|e| e.name.as_str()).collect();
            print(
                cli,
                &serde_json::json!({ "files": names, "attributions": manifest.attributions, "cache": cache_dir.display().to_string() }),
                || {
                    println!(
                        "Read {} files, made {}, into {}.",
                        names.len(),
                        manifest.created,
                        cache_dir.display()
                    );
                    for n in &names {
                        println!("  {n}");
                    }
                },
            )?;
            Ok(0)
        }
    }
}

/// The download cache: `$GNSS_CACHE`, or a `gnss` folder in the platform's
/// cache directory.
fn cache_dir(given: Option<&PathBuf>) -> Result<PathBuf, String> {
    if let Some(dir) = given {
        return Ok(dir.clone());
    }
    let env = |name: &str| {
        std::env::var_os(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if let Some(dir) = env("GNSS_CACHE") {
        return Ok(dir);
    }
    let base = if cfg!(windows) {
        env("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Caches"))
    } else {
        env("XDG_CACHE_HOME").or_else(|| env("HOME").map(|h| h.join(".cache")))
    };
    base.map(|b| b.join("gnss"))
        .ok_or_else(|| "no cache folder: set GNSS_CACHE or give --cache".to_owned())
}

fn fetch(cli: &Cli, args: &FetchArgs) -> Result<u8, Failure> {
    use gnss_api::fetch::gnss_fetch::{
        self as f, Cache, Fetcher, native::NativeHttp, products::Constellations,
    };
    use gnss_api::fetch::{Stations, purpose};
    let folder = Folder::open(&cli.project)?;
    let mut store = folder.store();
    let loaded = gnss_api::folder::load(&store).map_err(|e| e.to_string())?;
    let sources = f::read_sources(gnss_api::fetch::AU_PACK)?;
    let cache_dir = cache_dir(args.cache.as_ref())?;
    let (stamp, today, _) = now();
    let today = f::time::Day::parse(&today).ok_or("the system clock's date doesn't read")?;
    let options = f::Options {
        pad: args.pad * 60,
        precise: args.precise,
        constellations: if args.multi_gnss {
            Constellations::Multi
        } else {
            Constellations::Operational
        },
        antex: !args.no_antex,
    };
    let mut fetcher = Fetcher {
        sources: &sources,
        http: NativeHttp::new(cache_dir.join("partial")),
        cache: Cache::new(gnss_store::FsStore::new(&cache_dir)),
        today,
        // Whole seconds are plenty.
        now: format!("{}Z", stamp.get(..19).unwrap_or(&stamp)),
        options,
        route: match &args.relay {
            Some(relay) => f::Route::Via(relay.clone()),
            None => f::Route::Direct,
        },
        // Natively, every source is reached directly.
        app_data: None,
    };
    let stations = if args.cors.is_empty() {
        Stations::Nearest {
            count: args.nearest,
            within: args.within * 1000.0,
        }
    } else {
        Stations::Named(args.cors.clone())
    };
    let sites = match stations {
        Stations::Nearest { .. } => fetcher.sites()?,
        Stations::Named(_) => Vec::new(),
    };
    let (mut wants, mut problems) = gnss_api::fetch::wants(&loaded, &stations, &sites);
    if args.dry_run {
        let needs = f::plan(&wants, &options);
        let purposes: Vec<String> = needs.iter().map(purpose).collect();
        print(
            cli,
            &serde_json::json!({ "needs": purposes, "problems": problems }),
            || {
                for p in &purposes {
                    println!("Needs {p}");
                }
                for p in &problems {
                    println!("problem: {p}");
                }
            },
        )?;
        return Ok(if problems.is_empty() { 0 } else { TESTS_FAILED });
    }
    // The nearest stations with data in each window, falling back on the
    // next nearest.
    let chosen = fetcher.choose_stations(&mut wants);
    let needs = f::plan(&wants, &options);
    let mut report = fetcher.fetch(&needs);
    // Each CORS file's header against its station's site log.
    let site_logs = gnss_api::fetch::check_site_logs(&mut fetcher, &mut report);
    gnss_api::fetch::record(&mut store, &report, &site_logs).map_err(|e| e.to_string())?;
    problems.extend(
        report
            .problems
            .iter()
            .map(|p| format!("{}: {}", purpose(&p.need), p.message)),
    );
    let fetched: Vec<serde_json::Value> = report
        .fetched
        .iter()
        .map(|x| {
            let files: Vec<serde_json::Value> = x
                .files
                .iter()
                .map(
                    |e| serde_json::json!({ "name": e.name, "bytes": e.bytes, "sha256": e.sha256 }),
                )
                .collect();
            serde_json::json!({
                "purpose": purpose(&x.need),
                "files": files,
                "downloaded": x.downloaded,
                "note": x.note,
            })
        })
        .collect();
    print(
        cli,
        &serde_json::json!({
            "cache": cache_dir.display().to_string(),
            "fetched": fetched,
            "problems": problems,
            "stations": chosen,
            "site_logs": site_logs,
            "attributions": report.attributions,
        }),
        || {
            for note in &chosen {
                println!("note: {note}");
            }
            for x in &report.fetched {
                let how = if x.downloaded { "downloaded" } else { "cached" };
                let names: Vec<&str> = x.files.iter().map(|e| e.name.as_str()).collect();
                println!("{}: {} ({how})", purpose(&x.need), names.join(", "));
                if let Some(note) = &x.note {
                    println!("  note: {note}");
                }
            }
            for c in &site_logs {
                if c.findings.is_empty() {
                    println!(
                        "{}: the header agrees with {}'s site log",
                        c.file, c.station
                    );
                }
                for f in &c.findings {
                    println!("site log: {} ({}): {f}", c.file, c.station);
                }
            }
            for p in &problems {
                println!("problem: {p}");
            }
            println!(
                "Recorded in data.toml; the files are in {}.",
                cache_dir.display()
            );
            if !report.attributions.is_empty() {
                println!("Attribution:");
                for a in &report.attributions {
                    println!("  {a}");
                }
            }
        },
    )?;
    Ok(if problems.is_empty() { 0 } else { TESTS_FAILED })
}

fn qc(cli: &Cli, args: &QcArgs) -> Result<u8, Failure> {
    let open = |path: &PathBuf| {
        std::fs::File::open(path)
            .map(std::io::BufReader::new)
            .map_err(|e| format!("{}: {e}", path.display()))
    };
    let mut ephemerides = Vec::new();
    for path in &args.nav {
        ephemerides.extend(
            gnss_api::rinex::read_nav(open(path)?)
                .map_err(|e| format!("{}: {e}", path.display()))?,
        );
    }
    let orbits = (!args.nav.is_empty()).then(|| gnss_api::rinex::Orbits::new(ephemerides));
    let mut reports = Vec::new();
    for path in &args.files {
        let report = gnss_api::rinex::check_file(open(path)?, orbits.clone())
            .map_err(|e| format!("{}: {e}", path.display()))?;
        reports.push((path.display().to_string(), report));
    }
    let json: Vec<_> = reports
        .iter()
        .map(|(file, r)| serde_json::json!({ "file": file, "qc": r }))
        .collect();
    print(cli, &json, || {
        for (file, r) in &reports {
            println!("{file}: {} (RINEX {})", r.marker, r.version);
            let radome = r.radome.as_deref().unwrap_or("NONE");
            println!("  {} with {} {}", r.receiver, r.antenna, radome);
            if let Some(note) = &r.antenna_note {
                println!("  note: {note}");
            }
            if let Some([h, e, n]) = r.antenna_delta {
                println!("  antenna height {h:.4} m (east {e:.4}, north {n:.4})");
            }
            let span = match (&r.first, &r.last) {
                (Some(a), Some(b)) => format!("{a} to {b} {}", r.time_system),
                _ => "no epochs".to_owned(),
            };
            println!("  {span}");
            let completeness = r
                .expected_epochs
                .filter(|&e| e > 0)
                .map(|e| format!(" ({:.1}%)", 100.0 * r.epochs as f64 / e as f64))
                .unwrap_or_default();
            println!(
                "  {} of {} epochs{completeness} every {} s; {} gaps; {} events",
                r.epochs,
                r.expected_epochs.map_or("?".to_owned(), |e| e.to_string()),
                r.interval.map_or("?".to_owned(), |i| i.to_string()),
                r.gaps.len(),
                r.events
            );
            for (from, to, seconds) in &r.gaps {
                println!("    gap {from} to {to} ({seconds} s)");
            }
            if let Some(note) = &r.coverage_note {
                println!("  note: {note}");
            }
            for s in &r.systems {
                let mp: Vec<String> = s
                    .multipath
                    .iter()
                    .map(|(name, rms)| format!("{name} {rms:.3} m"))
                    .collect();
                let pair = s.pair.as_deref().map_or_else(
                    || "no dual-frequency pair, so no slips or multipath".to_owned(),
                    |p| {
                        format!(
                            "{p}: {} slips, {} flagged by the receiver, {} outliers",
                            s.slips, s.flagged, s.outliers
                        )
                    },
                );
                let mp = if mp.is_empty() {
                    String::new()
                } else {
                    format!(", {}", mp.join(", "))
                };
                println!("  {:8} {:3} satellites; {pair}{mp}", s.system, s.satellites);
                if let Some(c) = &s.coverage {
                    let ratio = c.ratio.map_or("nothing expected".to_owned(), |r| {
                        format!("{:.1}%", 100.0 * r)
                    });
                    println!(
                        "           above {}°: {} of {} satellite-epochs tracked ({ratio}), {} slips; {} below; {} unhealthy; {} with no ephemeris",
                        c.cutoff,
                        c.tracked,
                        c.expected,
                        c.slips_above_cutoff,
                        c.below_cutoff,
                        c.unhealthy,
                        c.no_ephemeris
                    );
                }
            }
        }
    })?;
    Ok(0)
}

fn import_rinex(cli: &Cli, args: &RinexArgs) -> Result<u8, Failure> {
    let folder = Folder::open(&cli.project)?;
    let mut files = Vec::new();
    for path in &args.files {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("{} isn't a file", path.display()))?;
        files.push((name, bytes));
    }
    let options = gnss_api::ingest::IngestOptions {
        mark: args.mark.clone(),
        join_s: args.join,
        ..Default::default()
    };
    let report = gnss_api::ingest::import_rinex(&mut folder.store(), &files, &options)
        .map_err(|e| e.to_string())?;
    print(cli, &report, || {
        for o in &report.occupations {
            let interval = o.interval.map_or("?".to_owned(), |i| i.to_string());
            println!(
                "{}: {} from {} to {}, {} epochs every {interval} s ({})",
                o.id,
                o.mark,
                o.start,
                o.end,
                o.epochs,
                o.files.join(", ")
            );
            for note in &o.notes {
                println!("  note: {note}");
            }
        }
        if !report.marks_added.is_empty() {
            println!(
                "Added to marks.toml: {} (check their names and positions)",
                report.marks_added.join(", ")
            );
        }
        for note in &report.notes {
            println!("note: {note}");
        }
    })?;
    Ok(0)
}

fn reconcile(cli: &Cli, args: &ReconcileArgs) -> Result<u8, Failure> {
    let folder = Folder::open(&cli.project)?;
    let mut store = folder.store();
    let mut fixed = Vec::new();
    if args.fix {
        let (stamp, _, _) = now();
        let now = gnss_api::reconcile::Timestamp::parse(&stamp).map_err(|e| e.to_string())?;
        let by = args.by.as_deref().unwrap_or_default();
        fixed = gnss_api::reconcile::fix(&mut store, by, now).map_err(|e| e.to_string())?;
    }
    let findings = gnss_api::reconcile::reconcile(&store).map_err(|e| e.to_string())?;
    print(
        cli,
        &serde_json::json!({ "overlays_written": fixed, "findings": findings }),
        || {
            for id in &fixed {
                println!("Wrote overlays/{id}.toml.");
            }
            if findings.is_empty() {
                println!("Every header agrees with its occupation.");
            }
            for f in &findings {
                let hint = if f.fixable {
                    " (gnss reconcile --fix corrects it)"
                } else {
                    ""
                };
                println!(
                    "{} {}: {}: the header says {}, the occupation {}{hint}",
                    f.occupation, f.file, f.what, f.header, f.occupation_value
                );
            }
        },
    )?;
    Ok(if findings.is_empty() { 0 } else { TESTS_FAILED })
}

/// A RINEX file's name once it's plain RINEX: `.crx` to `.rnx`, and a
/// RINEX 2 `.yyd` to `.yyo`.
fn plain_name(name: &str) -> String {
    if let Some(stem) = name.strip_suffix(".crx") {
        return if stem.ends_with(".rnx") {
            stem.to_owned()
        } else {
            format!("{stem}.rnx")
        };
    }
    let bytes = name.as_bytes();
    if let [.., b'.', a, b, b'd'] = bytes
        && a.is_ascii_digit()
        && b.is_ascii_digit()
    {
        return format!("{}o", &name[..name.len() - 1]);
    }
    name.to_owned()
}

fn export_rinex(cli: &Cli, args: &ExportRinexArgs) -> Result<u8, Failure> {
    let folder = Folder::open(&cli.project)?;
    let store = folder.store();
    let loaded = gnss_api::folder::load(&store).map_err(|e| e.to_string())?;
    let out_dir = args
        .output
        .clone()
        .unwrap_or_else(|| folder.results().join("rinex"));
    let mut files: Vec<&String> = loaded.occupations.iter().flat_map(|o| &o.rinex).collect();
    files.sort();
    files.dedup();
    let mut written = Vec::new();
    for file in files {
        let bytes =
            gnss_api::reconcile::corrected_rinex(&store, file).map_err(|e| e.to_string())?;
        let name = plain_name(file.trim_start_matches("raw/"));
        let path = out_dir.join(&name);
        folder.write(&path, &bytes)?;
        let corrected = loaded.overlays.iter().any(|o| &o.file == file);
        written.push(serde_json::json!({ "file": file, "written": path, "corrected": corrected }));
    }
    print(cli, &written, || {
        for w in &written {
            let note = if w["corrected"] == true {
                " (corrected)"
            } else {
                ""
            };
            println!("Wrote {}{note}.", w["written"].as_str().unwrap_or_default());
        }
    })?;
    Ok(0)
}

/// Prints `value` as JSON with `--json`, and otherwise runs `human`.
fn print<T: Serialize>(cli: &Cli, value: &T, human: impl FnOnce()) -> Result<(), Failure> {
    if cli.json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        human();
    }
    Ok(())
}

fn init(cli: &Cli, args: &InitArgs) -> Result<u8, Failure> {
    let frame = FrameId::parse(&args.frame)?;
    let epoch = args
        .epoch
        .map(CoordinateEpoch::from_decimal_year)
        .transpose()?;
    frame.resolve_epoch(epoch)?;
    let name = args.name.clone().unwrap_or_else(|| {
        args.dir
            .file_name()
            .map_or_else(|| "Survey".into(), |n| n.to_string_lossy().into_owned())
    });
    let project = Project::new(name, AdjustmentSettings::new(frame, epoch));
    let folder = Folder::create(&args.dir, &project)?;
    print(
        cli,
        &serde_json::json!({ "created": folder.root() }),
        || {
            println!(
                "Created project {:?} in {}, adjusting in {}.",
                project.name,
                args.dir.display(),
                project.adjustment.frame
            );
            println!("Next, in that folder: gnss import vectors <file> --format <format>");
        },
    )?;
    Ok(0)
}

/// Imports a file in `format`, or in the format it looks like, which must be
/// one of `allowed`.
fn import<F: ValueEnum + Clone + Send + Sync + 'static>(
    cli: &Cli,
    format: Option<Format>,
    allowed: &[Format],
    args: &ImportArgs<F>,
) -> Result<u8, Failure> {
    let folder = Folder::open(&cli.project)?;
    let file = &args.file;
    let bytes = std::fs::read(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| format!("{} isn't a file", file.display()))?;
    let guessed = format.is_none();
    let format = match format.or_else(|| project::guess_format(&name, &bytes)) {
        Some(f) if allowed.contains(&f) => f,
        Some(f) => {
            return Err(format!(
                "{name} looks like {}, which this command doesn't import; give --format",
                f.label()
            )
            .into());
        }
        None => {
            return Err(format!(
                "{name}: its format isn't clear from its contents or name; give --format"
            )
            .into());
        }
    };
    let entry = Import {
        file: format!("raw/{name}"),
        format,
        frame: args.frame.clone(),
        epoch: args.epoch.clone(),
        base: args.base.clone(),
        rover: args.rover.clone(),
    };
    // Read it now, so problems show at once.
    let survey = project::read_import(&entry, &bytes)?;
    folder.add_import(&entry, &bytes)?;
    let notes: Vec<String> = survey
        .notes
        .iter()
        .map(|n| format!("{}: {}", n.origin, n.message))
        .collect();
    print(
        cli,
        &serde_json::json!({
            "imported": entry.file,
            "format": format.name(),
            "guessed": guessed,
            "stations": survey.stations.len(),
            "measurements": survey.measurements.len(),
            "notes": notes,
        }),
        || {
            println!(
                "Imported {name} as {}{}: {} station{} and {} measurement{}.",
                format.name(),
                if guessed {
                    " (guessed from the file; give --format to choose)"
                } else {
                    ""
                },
                survey.stations.len(),
                if survey.stations.len() == 1 { "" } else { "s" },
                survey.measurements.len(),
                if survey.measurements.len() == 1 {
                    ""
                } else {
                    "s"
                }
            );
            for note in &notes {
                println!("  note: {note}");
            }
        },
    )?;
    Ok(0)
}

/// Loads and runs the project, with a datum override for this run.
fn run(cli: &Cli, datum: &AdjustArgs) -> Result<(Folder, Project, Run), Failure> {
    let folder = Folder::open(&cli.project)?;
    let mut project = folder.project()?;
    if datum.minimal {
        project.adjustment.datum = "minimal".into();
    } else if datum.constrained {
        project.adjustment.datum = "constrained".into();
    }
    let run = project::run(&project, &mut |import| folder.read_import(&import.file))?;
    Ok((folder, project, run))
}

fn print_summary(summary: &Summary) {
    println!(
        "{}: {} marks, {} observations, {} degrees of freedom, in {}{}.",
        summary.name,
        summary.marks,
        summary.observations,
        summary.degrees_of_freedom,
        summary.frame,
        summary
            .epoch
            .map(|e| format!(" at {e:.4}"))
            .unwrap_or_default()
    );
    match (&summary.global_test, summary.variance_factor) {
        (Some(g), Some(f)) => println!(
            "Global test {}: σ̂₀² {f:.3}, Ω/σ₀² {:.2} {} [{:.2}, {:.2}].",
            if g.passed { "passed" } else { "FAILED" },
            g.statistic,
            if g.passed { "within" } else { "outside" },
            g.lower,
            g.upper
        ),
        _ => println!("No redundancy, so nothing could be tested."),
    }
    if let Some((count, failed)) = summary.loops {
        println!("Loops: {count}, {failed} failed.");
    }
    if summary.suspects.is_empty() {
        println!("No observation failed a test.");
    } else {
        println!("{} test(s) failed, worst first:", summary.suspects.len());
        for s in summary.suspects.iter().take(20) {
            let element = if s.element > 1 {
                format!(" element {}", s.element)
            } else {
                String::new()
            };
            let test = if s.test == "vector" {
                format!("vector test {:.2} > {:.2}", s.statistic, s.critical)
            } else {
                format!("{} {:.2}, |w| > {:.2}", s.test, s.statistic, s.critical)
            };
            println!("  {}{element} ({}): {test}", s.observation, s.label);
        }
        if summary.suspects.len() > 20 {
            println!(
                "  ... and {} more in the report.",
                summary.suspects.len() - 20
            );
        }
    }
    for note in &summary.notes {
        println!("note: {note}");
    }
}

fn adjust(cli: &Cli, args: &AdjustArgs) -> Result<u8, Failure> {
    let (_, project, run) = run(cli, args)?;
    let summary = project::summary(&project, &run);
    print(cli, &summary, || print_summary(&summary))?;
    Ok(if summary.passed { 0 } else { TESTS_FAILED })
}

/// Now, in UTC, as an ISO 8601 date and time with milliseconds.
fn now() -> (String, String, String) {
    let now = hifitime::Epoch::now().unwrap_or_else(|_| hifitime::Epoch::from_unix_seconds(0.0));
    let (y, mo, d, h, mi, s, ns) = now.to_gregorian_utc();
    (
        format!(
            "{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{:03}Z",
            ns / 1_000_000
        ),
        format!("{y:04}-{mo:02}-{d:02}"),
        format!("{h:02}:{mi:02}:{s:02}"),
    )
}

fn report(cli: &Cli, args: &ReportArgs) -> Result<u8, Failure> {
    let (folder, project, run) = run(cli, &args.datum)?;
    let summary = project::summary(&project, &run);
    let (_, date, time) = now();
    // Data from outside the project, if it fetched any.
    let data = match std::fs::read_to_string(folder.root().join("data.toml")) {
        Ok(text) => {
            Some(gnss_model::DataFiles::from_toml(&text).map_err(|e| format!("data.toml: {e}"))?)
        }
        Err(_) => None,
    };
    let html =
        project::report_with_data(&project, &run, data.as_ref(), &format!("{date} {time} UTC"));
    let path = args
        .output
        .clone()
        .unwrap_or_else(|| folder.results().join("report.html"));
    folder.write(&path, html.as_bytes())?;
    print(
        cli,
        &serde_json::json!({ "written": path, "summary": summary }),
        || {
            print_summary(&summary);
            println!("Wrote {}.", path.display());
        },
    )?;
    Ok(if summary.passed { 0 } else { TESTS_FAILED })
}

fn export_results(cli: &Cli, args: &ExportArgs) -> Result<u8, Failure> {
    let (folder, _, run) = run(cli, &args.datum)?;
    let format = match args.format {
        ExportFormat::Dynaml => project::ExportFormat::Dynaml,
        ExportFormat::Starnet => project::ExportFormat::Starnet,
        ExportFormat::Landxml => project::ExportFormat::Landxml,
        ExportFormat::Gpkg => project::ExportFormat::Gpkg,
        ExportFormat::Csv => project::ExportFormat::Csv,
    };
    let (stamp, ..) = now();
    let files = project::export(&run, format, args.ellipse_scale, &stamp)?;
    // --output is the file for a format that writes one, and the folder for
    // one that writes two.
    let mut written = Vec::new();
    for file in files.iter() {
        let path = match &args.output {
            Some(output) if files.len() == 1 => output.clone(),
            Some(output) => output.join(&file.name),
            None => folder.results().join(&file.name),
        };
        folder.write(&path, &file.bytes)?;
        written.push(path);
    }
    print(cli, &serde_json::json!({ "written": written }), || {
        for path in &written {
            println!("Wrote {}.", path.display());
        }
    })?;
    Ok(0)
}

fn exclude(cli: &Cli, args: &ExcludeArgs) -> Result<u8, Failure> {
    if args.reason.trim().is_empty() {
        return Err("an exclusion needs a reason".into());
    }
    // Check it's there to exclude: an edit to nothing would stop the project
    // adjusting until project.toml was fixed by hand.
    let (folder, _, run) = run(cli, &AdjustArgs::default())?;
    if !run
        .built
        .network
        .observations()
        .iter()
        .any(|o| o.id == args.observation)
    {
        return Err(format!(
            "there's no observation {} in the project; use the identifier the report shows (file:line)",
            args.observation
        )
        .into());
    }
    if let Some(e) = run
        .adjustment
        .edits()
        .iter()
        .find(|e| e.observation == args.observation)
    {
        return Err(format!(
            "{} is already edited ({}); change its [[edit]] in project.toml instead",
            args.observation, e.reason
        )
        .into());
    }
    let edit = EditRecord {
        observation: args.observation.clone(),
        action: "exclude".into(),
        factor: None,
        reason: args.reason.clone(),
    };
    folder.add_edit(&edit)?;
    print(
        cli,
        &serde_json::json!({ "excluded": edit.observation }),
        || {
            println!(
                "Excluded {} ({}). Adjust again to see the effect; to include it again, remove its \
             [[edit]] from project.toml.",
                edit.observation, edit.reason
            );
        },
    )?;
    Ok(0)
}
