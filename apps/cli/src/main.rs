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
                  failed or there was no redundancy to test with (adjust and report)."
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
    /// GNSS vectors: DynaML, STAR*NET, RTKLIB or CSV.
    Vectors(ImportArgs<VectorFormat>),
    /// Control: DynaML stations, CSV or SINEX.
    Control(ImportArgs<ControlFormat>),
}

#[derive(Args)]
struct ImportArgs<F: ValueEnum + Clone + Send + Sync + 'static> {
    /// The file.
    file: PathBuf,
    /// Its format. Without one, it's guessed from the file's contents and
    /// name, and the guess is shown.
    #[arg(long)]
    format: Option<F>,
    /// The frame of its contents, for formats that don't say (STAR*NET, CSV
    /// without a frame column, RTKLIB, SINEX without one in its comments).
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
                VectorFormat::Rtklib => Format::Rtklib,
                VectorFormat::Csv => Format::CsvVectors,
            });
            let allowed = [
                Format::Dynaml,
                Format::Starnet,
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
        Command::Adjust(args) => adjust(cli, args),
        Command::Report(args) => report(cli, args),
        Command::Export(args) => export_results(cli, args),
        Command::Exclude(args) => exclude(cli, args),
    }
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
    let run = project::run(&project, &mut |import| folder.read_raw(&import.file))?;
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
    let html = project::report(&project, &run, &format!("{date} {time} UTC"));
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
