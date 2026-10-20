//! A second-opinion project (M3): the settings, the files imported and the
//! person's edits, as `project.toml` holds them, and running it: reading the
//! imports, building the network, adjusting it and summarising the result.
//!
//! The core never touches the file system: callers hand over each import's
//! bytes, so the CLI and the web page run projects the same way. The full,
//! occupation-first project model replaces this in M5.

use gnss_adjust::closures::Closures;
use gnss_adjust::{Action, Adjustment, Edit, Options, Scaling, Subject};
use gnss_datum::{CoordinateEpoch, FrameId, TransformationGraph};
use gnss_formats::{FormatError, Reference, Survey, csv, decode, dynaml, rtklib, sinex, starnet};
use serde::{Deserialize, Serialize};

use crate::build::{BuildError, Built, DatumChoice, Settings, build};

/// The project file's schema version.
pub const SCHEMA: u32 = 1;

/// A project, as `project.toml` holds it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// The schema version, [`SCHEMA`].
    pub schema: u32,
    /// The project's name, for the report.
    pub name: String,
    /// How to adjust.
    pub adjustment: AdjustmentSettings,
    /// The files imported, in order.
    #[serde(default, rename = "import", skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<Import>,
    /// The person's exclusions and re-weightings, with their reasons.
    #[serde(default, rename = "edit", skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<EditRecord>,
}

/// How to adjust.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdjustmentSettings {
    /// The frame to adjust in, such as "GDA2020".
    pub frame: String,
    /// The coordinate epoch, as a decimal year, for a dynamic frame.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<f64>,
    /// "constrained" (control as imported), "minimal" (inner constraints) or
    /// "fixed:NAME" (minimal, holding one mark).
    #[serde(default = "constrained")]
    pub datum: String,
    /// "a-priori" or "a-posteriori": the variance factor uncertainties use.
    #[serde(default = "a_priori")]
    pub uncertainty: String,
    /// The w-test's level, α₀.
    #[serde(default = "alpha0")]
    pub alpha0: f64,
    /// The tests' power, β₀.
    #[serde(default = "beta0")]
    pub beta0: f64,
    /// The global test's level.
    #[serde(default = "global_alpha")]
    pub global_alpha: f64,
}

fn constrained() -> String {
    "constrained".into()
}
fn a_priori() -> String {
    "a-priori".into()
}
fn alpha0() -> f64 {
    0.001
}
fn beta0() -> f64 {
    0.8
}
fn global_alpha() -> f64 {
    0.05
}

impl AdjustmentSettings {
    /// Settings for a frame, with the defaults: constrained, a priori, and
    /// the ICSM tests (α₀ 0.1%, β₀ 80%, a 5% global test).
    pub fn new(frame: FrameId, epoch: Option<CoordinateEpoch>) -> Self {
        Self {
            frame: frame.name().into(),
            epoch: epoch.map(CoordinateEpoch::decimal_year),
            datum: constrained(),
            uncertainty: a_priori(),
            alpha0: alpha0(),
            beta0: beta0(),
            global_alpha: global_alpha(),
        }
    }
}

/// The formats files can be imported from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Format {
    /// DynaML stations and measurements.
    Dynaml,
    /// STAR*NET GPS vectors.
    Starnet,
    /// An RTKLIB solution.
    Rtklib,
    /// The CSV vector format.
    CsvVectors,
    /// The CSV control format.
    CsvControl,
    /// A SINEX solution.
    Sinex,
}

impl Format {
    /// Every format, with its name.
    pub const ALL: [(Format, &'static str); 6] = [
        (Format::Dynaml, "dynaml"),
        (Format::Starnet, "starnet"),
        (Format::Rtklib, "rtklib"),
        (Format::CsvVectors, "csv-vectors"),
        (Format::CsvControl, "csv-control"),
        (Format::Sinex, "sinex"),
    ];

    /// The format's name in project files.
    pub fn name(self) -> &'static str {
        Format::ALL
            .iter()
            .find(|(f, _)| *f == self)
            .map_or("", |(_, n)| n)
    }

    /// A format from its name.
    pub fn parse(name: &str) -> Option<Format> {
        Format::ALL
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(f, _)| *f)
    }
}

/// A file imported into the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    /// The file, relative to the project folder, such as "raw/vectors.xml".
    pub file: String,
    /// Its format.
    pub format: Format,
    /// The frame of its contents, for formats that don't say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
    /// The epoch of its contents, as a decimal year or date, for formats that don't say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<String>,
    /// For an RTKLIB solution: the base's mark name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// For an RTKLIB solution: the rover's mark name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rover: Option<String>,
}

impl Import {
    /// The file's name without its folder, as observations' identifiers use it.
    pub fn name(&self) -> &str {
        self.file.rsplit(['/', '\\']).next().unwrap_or(&self.file)
    }
}

/// An exclusion or re-weighting, with the person's reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRecord {
    /// The observation's identifier, such as "vectors.xml:21".
    pub observation: String,
    /// "exclude" or "reweight".
    pub action: String,
    /// For a re-weighting: the factor the covariance is multiplied by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factor: Option<f64>,
    /// Why.
    pub reason: String,
}

/// A problem with a project.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ProjectError {
    /// The project file can't be read or written.
    #[error("project.toml: {0}")]
    File(String),
    /// A setting that can't be used.
    #[error("project.toml: {0}")]
    Setting(String),
    /// An import that can't be read.
    #[error("{0}")]
    Import(String),
    /// A file that can't be parsed.
    #[error("{0}")]
    Format(#[from] FormatError),
    /// The network can't be built.
    #[error("{0}")]
    Build(#[from] BuildError),
    /// The network can't be adjusted.
    #[error("{0}")]
    Adjust(#[from] gnss_adjust::AdjustError),
}

impl Project {
    /// A new project.
    pub fn new(name: impl Into<String>, adjustment: AdjustmentSettings) -> Self {
        Self {
            schema: SCHEMA,
            name: name.into(),
            adjustment,
            imports: Vec::new(),
            edits: Vec::new(),
        }
    }

    /// A project from `project.toml`'s text.
    pub fn from_toml(text: &str) -> Result<Self, ProjectError> {
        let project: Project =
            toml::from_str(text).map_err(|e| ProjectError::File(e.to_string()))?;
        if project.schema != SCHEMA {
            return Err(ProjectError::File(format!(
                "schema {} isn't one this version reads ({SCHEMA})",
                project.schema
            )));
        }
        Ok(project)
    }

    /// The project as `project.toml`'s text.
    pub fn to_toml(&self) -> Result<String, ProjectError> {
        toml::to_string(self).map_err(|e| ProjectError::File(e.to_string()))
    }

    /// The frame and epoch to adjust in.
    pub fn reference(&self) -> Result<(FrameId, Option<CoordinateEpoch>), ProjectError> {
        let frame = FrameId::parse(&self.adjustment.frame)
            .map_err(|e| ProjectError::Setting(e.to_string()))?;
        let epoch = self
            .adjustment
            .epoch
            .map(CoordinateEpoch::from_decimal_year)
            .transpose()
            .map_err(|e| ProjectError::Setting(e.to_string()))?;
        frame
            .resolve_epoch(epoch)
            .map_err(|e| ProjectError::Setting(format!("the adjustment's frame: {e}")))?;
        Ok((frame, epoch))
    }

    /// The datum setting.
    pub fn datum(&self) -> Result<DatumChoice, ProjectError> {
        match self.adjustment.datum.as_str() {
            "constrained" => Ok(DatumChoice::Constrained),
            "minimal" => Ok(DatumChoice::Minimal),
            other => match other.strip_prefix("fixed:") {
                Some(mark) if !mark.trim().is_empty() => {
                    Ok(DatumChoice::FixedMark(mark.trim().to_owned()))
                }
                _ => Err(ProjectError::Setting(format!(
                    "datum should be constrained, minimal or fixed:MARK, not {other:?}"
                ))),
            },
        }
    }

    /// The uncertainty setting.
    pub fn scaling(&self) -> Result<Scaling, ProjectError> {
        match self.adjustment.uncertainty.as_str() {
            "a-priori" => Ok(Scaling::APriori),
            "a-posteriori" => Ok(Scaling::APosteriori),
            other => Err(ProjectError::Setting(format!(
                "uncertainty should be a-priori or a-posteriori, not {other:?}"
            ))),
        }
    }

    /// The edits as the adjustment takes them.
    pub fn edits(&self) -> Result<Vec<Edit>, ProjectError> {
        self.edits
            .iter()
            .map(|e| {
                let action = match (e.action.as_str(), e.factor) {
                    ("exclude", None) => Action::Exclude,
                    ("reweight", Some(f)) if f.is_finite() && f > 0.0 => Action::Reweight(f),
                    _ => {
                        return Err(ProjectError::Setting(format!(
                            "the edit to {} should be action = \"exclude\", or \"reweight\" with a \
                             positive factor",
                            e.observation
                        )));
                    }
                };
                if e.reason.trim().is_empty() {
                    return Err(ProjectError::Setting(format!(
                        "the edit to {} needs a reason",
                        e.observation
                    )));
                }
                Ok(Edit {
                    observation: e.observation.clone(),
                    action,
                    reason: e.reason.clone(),
                })
            })
            .collect()
    }
}

/// Reads one import's bytes.
pub fn read_import(import: &Import, bytes: &[u8]) -> Result<Survey, ProjectError> {
    let text = decode(bytes);
    let name = import.name();
    let frame = import
        .frame
        .as_deref()
        .map(FrameId::parse)
        .transpose()
        .map_err(|e| ProjectError::Import(format!("{name}: {e}")))?;
    let epoch = import
        .epoch
        .as_deref()
        .map(|e| gnss_formats::parse_epoch("the import's epoch", e))
        .transpose()
        .map_err(|e| ProjectError::Import(format!("{name}: {e}")))?;
    let reference = frame.map(|frame| Reference {
        frame,
        epoch: if frame.is_static() { None } else { epoch },
    });
    let survey = match import.format {
        Format::Dynaml => dynaml::read(name, &text)?,
        Format::Starnet => {
            let reference = reference.ok_or_else(|| {
                ProjectError::Import(format!(
                    "{name}: STAR*NET files don't say which frame and epoch their vectors are in; \
                     give the import a frame (and an epoch for a dynamic frame)"
                ))
            })?;
            if !reference.frame.is_static() && reference.epoch.is_none() {
                return Err(ProjectError::Import(format!(
                    "{name}: {} is a dynamic frame, so the import needs an epoch",
                    reference.frame
                )));
            }
            starnet::read(name, &text, reference)?
        }
        Format::Rtklib => rtklib::read(
            name,
            &text,
            frame,
            &rtklib::Names {
                base: import.base.clone(),
                rover: import.rover.clone(),
            },
        )?,
        Format::CsvVectors => csv::read_vectors(name, &text, reference)?,
        Format::CsvControl => csv::read_control(name, &text, reference)?,
        Format::Sinex => sinex::read(name, &text, frame)?,
    };
    Ok(survey)
}

/// An input file's details, for provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InputFile {
    /// Its name.
    pub name: String,
    /// The format it was read as.
    pub format: String,
    /// Its SHA-256.
    pub sha256: String,
    /// Its size in bytes.
    pub bytes: usize,
}

/// A project run: what was read, built and adjusted.
#[derive(Debug, Clone)]
pub struct Run {
    /// The network as built, with what was done to get it.
    pub built: Built,
    /// The adjustment.
    pub adjustment: Adjustment,
    /// Loop closures and repeat baselines.
    pub closures: Option<Closures>,
    /// The input files.
    pub files: Vec<InputFile>,
    /// How uncertainties are scaled.
    pub scaling: Scaling,
}

/// Runs a project: reads each import, with `read` giving its bytes, then
/// builds and adjusts the network.
pub fn run(
    project: &Project,
    read: &mut dyn FnMut(&Import) -> Result<Vec<u8>, String>,
) -> Result<Run, ProjectError> {
    if project.imports.is_empty() {
        return Err(ProjectError::Setting(
            "nothing is imported yet: import vectors and control first".into(),
        ));
    }
    let (frame, epoch) = project.reference()?;
    let datum = project.datum()?;
    let scaling = project.scaling()?;
    let mut survey = Survey::default();
    let mut files = Vec::new();
    for import in &project.imports {
        let bytes =
            read(import).map_err(|e| ProjectError::Import(format!("{}: {e}", import.file)))?;
        survey.extend(read_import(import, &bytes)?);
        files.push(InputFile {
            name: import.name().to_owned(),
            format: import.format.name().to_owned(),
            sha256: gnss_store::sha256_hex(&bytes),
            bytes: bytes.len(),
        });
    }
    let settings = Settings {
        frame,
        epoch,
        datum,
    };
    let built = build(&survey, &settings, TransformationGraph::builtin())?;
    let mut edits = built.edits.clone();
    edits.extend(project.edits()?);
    let options = Options {
        alpha0: project.adjustment.alpha0,
        beta0: project.adjustment.beta0,
        global_alpha: project.adjustment.global_alpha,
        edits,
        ..Options::default()
    };
    let adjustment = built.network.adjust(&built.datum, &options)?;
    let closures = built.network.closures(&options).ok();
    Ok(Run {
        built,
        adjustment,
        closures,
        files,
        scaling,
    })
}

/// The global test, summarised.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GlobalTestSummary {
    /// Ω/σ₀².
    pub statistic: f64,
    /// The lower bound.
    pub lower: f64,
    /// The upper bound.
    pub upper: f64,
    /// Whether it passed.
    pub passed: bool,
}

/// A failed test, summarised.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SuspectSummary {
    /// The observation's identifier, or the set-up's mark.
    pub observation: String,
    /// The element, counting from 1, or 0 for a set-up.
    pub element: usize,
    /// A label, such as "A to B".
    pub label: String,
    /// "w east", "w north", "w up" or "vector".
    pub test: String,
    /// The statistic.
    pub statistic: f64,
    /// Its critical value.
    pub critical: f64,
    /// How far it failed.
    pub excess: f64,
}

/// A run's outcome, for `--json` and the web page.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// The project's name.
    pub name: String,
    /// The frame adjusted in.
    pub frame: String,
    /// The epoch, for a dynamic frame.
    pub epoch: Option<f64>,
    /// How many marks.
    pub marks: usize,
    /// How many observations.
    pub observations: usize,
    /// Degrees of freedom.
    pub degrees_of_freedom: usize,
    /// σ̂₀², if there's redundancy.
    pub variance_factor: Option<f64>,
    /// The global test.
    pub global_test: Option<GlobalTestSummary>,
    /// Failed tests, worst first.
    pub suspects: Vec<SuspectSummary>,
    /// Loops: how many, and how many failed.
    pub loops: Option<(usize, usize)>,
    /// What the readers and the build skipped or assumed.
    pub notes: Vec<String>,
    /// True if the global test passed and no test of an observation failed.
    pub passed: bool,
}

/// A run's outcome.
pub fn summary(project: &Project, run: &Run) -> Summary {
    let a = &run.adjustment;
    let n = &run.built.network;
    let statistics = a.statistics();
    let criteria = a.criteria();
    let global = statistics.global_test.as_ref().map(|g| GlobalTestSummary {
        statistic: g.statistic,
        lower: g.lower,
        upper: g.upper,
        passed: g.passed,
    });
    let suspects: Vec<SuspectSummary> = a
        .suspects()
        .iter()
        .map(|s| {
            let test = match s.component {
                Some(c) => format!("w {}", ["east", "north", "up"][c]),
                None => "vector".into(),
            };
            let critical = if s.component.is_some() {
                criteria.w
            } else {
                criteria.vector
            };
            let (observation, element, label) = match s.subject {
                Subject::Element {
                    observation,
                    element,
                } => (
                    n.observations()[observation].id.clone(),
                    element + 1,
                    run.built
                        .sources
                        .get(observation)
                        .map(|s| s.label.clone())
                        .unwrap_or_default(),
                ),
                Subject::Setup(setup) => {
                    let mark = n.setups()[setup].mark;
                    (
                        format!("set-up {}", setup + 1),
                        0,
                        format!("set-up over {}", n.marks()[mark].name),
                    )
                }
            };
            SuspectSummary {
                observation,
                element,
                label,
                test,
                statistic: s.statistic,
                critical,
                excess: s.excess,
            }
        })
        .collect();
    let passed = global.as_ref().is_some_and(|g| g.passed) && suspects.is_empty();
    Summary {
        name: project.name.clone(),
        frame: n.frame().name().into(),
        epoch: (!n.frame().is_static()).then(|| n.epoch().decimal_year()),
        marks: n.marks().len(),
        observations: n.observations().len(),
        degrees_of_freedom: statistics.degrees_of_freedom,
        variance_factor: statistics.variance_factor,
        global_test: global,
        suspects,
        loops: run
            .closures
            .as_ref()
            .map(|c| (c.loops.len(), c.loops.iter().filter(|l| !l.passed).count())),
        notes: run
            .built
            .notes
            .iter()
            .map(|n| format!("{}: {}", n.origin, n.message))
            .collect(),
        passed,
    }
}

/// The run's HTML report. `generated` is when, as the caller wants it shown.
pub fn report(project: &Project, run: &Run, generated: &str) -> String {
    let built = &run.built;
    let fixed = match &built.datum {
        gnss_adjust::Datum::Constrained { fixed } => fixed.len(),
        _ => 0,
    };
    let positions = built
        .network
        .observations()
        .iter()
        .flat_map(|o| &o.elements)
        .filter(|e| matches!(e, gnss_adjust::Element::Position { .. }))
        .count();
    let count = |k: usize, what: &str| format!("{k} {what}{}", if k == 1 { "" } else { "s" });
    let datum = match &built.datum {
        gnss_adjust::Datum::Constrained { .. } => match (fixed, positions) {
            (0, p) => format!(
                "Constrained by {} (weighted control).",
                count(p, "observed position")
            ),
            (f, 0) => format!("Constrained by {}.", count(f, "fixed mark")),
            (f, p) => format!(
                "Constrained by {} and {} (weighted control).",
                count(f, "fixed mark"),
                count(p, "observed position")
            ),
        },
        gnss_adjust::Datum::FixedMark(m) => format!(
            "Minimally constrained, holding {} at its imported position.",
            built.network.marks()[*m].name
        ),
        gnss_adjust::Datum::Inner => {
            "Minimally constrained by inner constraints: the vectors on their own.".into()
        }
    };
    let transformations = built
        .transformations
        .iter()
        .map(|t| {
            let from = match (t.from.frame.is_static(), t.from.epoch) {
                (false, Some(e)) => format!("{} at {e}", t.from.frame),
                _ => t.from.frame.to_string(),
            };
            format!(
                "{} measurement{} from {from}: {}",
                t.measurements,
                if t.measurements == 1 { "" } else { "s" },
                t.steps.join("; then ")
            )
        })
        .collect();
    let settings = vec![
        ("Datum setting".to_owned(), project.adjustment.datum.clone()),
        (
            "Tests".to_owned(),
            format!(
                "α₀ {}, β₀ {}, global test at {}",
                project.adjustment.alpha0,
                project.adjustment.beta0,
                project.adjustment.global_alpha
            ),
        ),
    ];
    let input = gnss_report::Input {
        title: project.name.clone(),
        network: &built.network,
        adjustment: &run.adjustment,
        closures: run.closures.as_ref(),
        scaling: run.scaling,
        datum,
        observations: built
            .sources
            .iter()
            .map(|s| gnss_report::ObservationInfo {
                label: s.label.clone(),
                source: format!("{}:{}", s.origin.file, s.origin.line),
            })
            .collect(),
        files: run
            .files
            .iter()
            .map(|f| gnss_report::FileInfo {
                name: f.name.clone(),
                format: f.format.clone(),
                sha256: f.sha256.clone(),
                bytes: f.bytes,
            })
            .collect(),
        transformations,
        notes: built
            .notes
            .iter()
            .map(|n| format!("{}: {}", n.origin, n.message))
            .collect(),
        settings,
        software: format!(
            "gnss {} ({})",
            crate::VERSION,
            crate::BuildTarget::current().as_str()
        ),
        generated: generated.to_owned(),
    };
    gnss_report::html(&input)
}
