//! Standards evaluation and report generation.
//!
//! See `docs/ARCHITECTURE.md` for this crate's place in the design.
//!
//! So far this is the M3 HTML report: one self-contained page (no scripts,
//! no external resources) with the network summary, the statistics and the
//! outlier table, coordinates with SP1's positional uncertainty, relative
//! uncertainty between observed pairs, loop closures, every observation's
//! residuals and tests, and the provenance of the inputs. It's rendered from
//! plain inputs, so any front end can produce it, and the same inputs give
//! the same bytes.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use gnss_adjust::closures::Closures;
use gnss_adjust::{Action, Adjustment, Element, Network, Scaling, Subject};
use gnss_datum::tm::{MgaGrid, to_mga};

/// What a report shows besides the adjustment itself.
#[derive(Debug, Clone)]
pub struct Input<'a> {
    /// The project's name.
    pub title: String,
    /// The network adjusted.
    pub network: &'a Network,
    /// Its adjustment.
    pub adjustment: &'a Adjustment,
    /// Its loop closures and repeat baselines, if computed.
    pub closures: Option<&'a Closures>,
    /// Whether uncertainties use the a priori or a posteriori variance factor.
    pub scaling: Scaling,
    /// How the datum was defined, in a sentence.
    pub datum: String,
    /// For each observation, a label and where it came from.
    pub observations: Vec<ObservationInfo>,
    /// The input files.
    pub files: Vec<FileInfo>,
    /// The transformations applied to bring measurements into the network's
    /// frame and epoch, one line each.
    pub transformations: Vec<String>,
    /// What the readers and the build skipped or assumed, one line each.
    pub notes: Vec<String>,
    /// Other settings worth recording, as (name, value).
    pub settings: Vec<(String, String)>,
    /// The software and build, such as "gnss 0.1.0 (native)".
    pub software: String,
    /// When the report was made, as the caller wants it shown.
    pub generated: String,
}

/// An observation's label and source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservationInfo {
    /// A short label, such as "A to B".
    pub label: String,
    /// Where it came from, such as "vectors.xml:21".
    pub source: String,
}

/// An input file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Its name.
    pub name: String,
    /// The format it was read as.
    pub format: String,
    /// Its SHA-256, in hexadecimal.
    pub sha256: String,
    /// Its size in bytes.
    pub bytes: usize,
}

/// `text` with HTML's special characters escaped.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Degrees as ±DDD°MM′SS.SSSSS″.
fn dms(degrees: f64) -> String {
    let sign = if degrees < 0.0 { "−" } else { "" };
    // Hundred-thousandths of a second, rounded once, then split.
    let units = (degrees.abs() * 3600.0 * 1e5).round();
    let d = (units / 3.6e8).floor();
    let rest = units - d * 3.6e8;
    let m = (rest / 6e6).floor();
    let s = (rest - m * 6e6) / 1e5;
    format!("{sign}{d:.0}°{m:02.0}′{s:08.5}″")
}

fn mm(metres: f64) -> String {
    format!("{:.1}", metres * 1000.0)
}

fn optional(v: Option<f64>, decimals: usize) -> String {
    v.map(|v| format!("{v:.decimals$}"))
        .unwrap_or_else(|| "–".into())
}

const STYLE: &str = "
:root { --ink: #1d2733; --muted: #5a6776; --line: #d5dbe2; --band: #f4f6f8; --bg: #ffffff;
  --pass: #1f7a3a; --pass-bg: #e6f4ea; --fail: #a4262c; --fail-bg: #fbe9ea; --accent: #24527a; }
@media (prefers-color-scheme: dark) {
  :root { --ink: #e3e8ee; --muted: #9aa7b5; --line: #3a4552; --band: #1e252d; --bg: #151a20;
    --pass: #7fd39a; --pass-bg: #18331f; --fail: #f19a9f; --fail-bg: #3b1d20; --accent: #8fb8e0; }
}
* { box-sizing: border-box; }
body { margin: 0 auto; max-width: 1320px; padding: 24px 16px 64px; background: var(--bg);
  color: var(--ink); font: 15px/1.5 system-ui, -apple-system, 'Segoe UI', Roboto, sans-serif; }
h1 { font-size: 1.7rem; margin: 0 0 4px; }
h2 { font-size: 1.25rem; margin: 40px 0 8px; padding-bottom: 4px; border-bottom: 2px solid var(--line); }
h3 { font-size: 1.05rem; margin: 24px 0 8px; }
p, li { max-width: 80ch; }
.meta, .note { color: var(--muted); }
nav { margin: 16px 0 0; display: flex; flex-wrap: wrap; gap: 4px 16px; }
nav a { color: var(--accent); }
.verdict { margin: 16px 0; padding: 12px 16px; border-radius: 6px; font-weight: 600; }
.pass { color: var(--pass); background: var(--pass-bg); }
.fail { color: var(--fail); background: var(--fail-bg); }
dl { display: grid; grid-template-columns: max-content 1fr; gap: 4px 24px; margin: 8px 0; }
dt { color: var(--muted); }
dd { margin: 0; }
.scroll { overflow-x: auto; margin: 8px 0; }
table { border-collapse: collapse; font-size: 0.9rem; width: 100%; }
th, td { padding: 4px 6px; border-bottom: 1px solid var(--line); text-align: left; white-space: nowrap; }
th { background: var(--band); font-weight: 600; position: sticky; top: 0; }
td.n, th.n { text-align: right; font-variant-numeric: tabular-nums; }
tr.flag td { background: var(--fail-bg); }
code { font-size: 0.85em; word-break: break-all; }
@media print { body { max-width: none; font-size: 10pt; } nav { display: none; } th { position: static; } }
";

/// Renders the report as one HTML page.
pub fn html(input: &Input<'_>) -> String {
    let a = input.adjustment;
    let n = input.network;
    let statistics = a.statistics();
    let criteria = a.criteria();
    let suspects = a.suspects();
    let mut out = String::with_capacity(64 * 1024);
    let _ = write!(
        out,
        "<!doctype html>\n<html lang=\"en-AU\">\n<head>\n<meta charset=\"utf-8\">\n\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{}</title>\n<style>{STYLE}</style>\n</head>\n<body>\n<header>\n<h1>{}</h1>\n\
         <p class=\"meta\">Adjustment report, made {} by {}.</p>\n\
         <nav><a href=\"#summary\">Summary</a><a href=\"#tests\">Tests</a>\
         <a href=\"#suspects\">Suspects</a><a href=\"#coordinates\">Coordinates</a>\
         <a href=\"#relative\">Relative uncertainty</a><a href=\"#closures\">Loop closures</a>\
         <a href=\"#observations\">Observations</a><a href=\"#provenance\">Provenance</a></nav>\n\
         </header>\n",
        escape(&input.title),
        escape(&input.title),
        escape(&input.generated),
        escape(&input.software)
    );

    // Summary.
    let elements: usize = n.observations().iter().map(|o| o.elements.len()).sum();
    let global = statistics.global_test.as_ref();
    let passed = global.is_some_and(|g| g.passed);
    let verdict = match (global, suspects.len()) {
        (None, _) => (
            "fail",
            "The network has no redundancy, so nothing could be tested.".to_owned(),
        ),
        (Some(g), 0) if g.passed => (
            "pass",
            "The global test passed and no observation is suspect.".to_owned(),
        ),
        (Some(g), k) => (
            if g.passed && k == 0 { "pass" } else { "fail" },
            format!(
                "The global test {}; {k} test{} of observations failed.",
                if g.passed { "passed" } else { "failed" },
                if k == 1 { "" } else { "s" }
            ),
        ),
    };
    let epoch = if n.frame().is_static() {
        String::new()
    } else {
        format!(" at epoch {}", n.epoch())
    };
    let _ = write!(
        out,
        "<section id=\"summary\">\n<h2>Summary</h2>\n<div class=\"verdict {}\">{}</div>\n<dl>\n\
         <dt>Frame</dt><dd>{}{epoch}</dd>\n<dt>Datum</dt><dd>{}</dd>\n\
         <dt>Marks</dt><dd>{}</dd>\n<dt>Observations</dt><dd>{} ({elements} vectors or positions)</dd>\n\
         <dt>Degrees of freedom</dt><dd>{}</dd>\n<dt>Variance factor σ̂₀²</dt><dd>{}</dd>\n\
         <dt>Uncertainties</dt><dd>{}</dd>\n</dl>\n</section>\n",
        verdict.0,
        escape(&verdict.1),
        escape(n.frame().name()),
        escape(&input.datum),
        n.marks().len(),
        n.observations().len(),
        statistics.degrees_of_freedom,
        optional(statistics.variance_factor, 3),
        match input.scaling {
            Scaling::APriori => "scaled by the a priori variance factor",
            Scaling::APosteriori => "scaled by the a posteriori variance factor σ̂₀²",
        }
    );

    // Tests.
    let _ = write!(
        out,
        "<section id=\"tests\">\n<h2>Tests</h2>\n<p>The w-test's level α₀ and power β₀ set every \
         test's minimal detectable bias by the B-method; the global test has its own level.</p>\n\
         <dl>\n<dt>Global test</dt><dd>{}</dd>\n\
         <dt>w-test</dt><dd>|w| ≤ {:.2} (α₀ {}, β₀ {}, λ₀ {:.2})</dd>\n\
         <dt>Vector test</dt><dd>T ≤ {:.2} (χ²(3) at {:.3}%)</dd>\n</dl>\n</section>\n",
        match global {
            Some(g) => format!(
                "Ω/σ₀² = {:.2} {} [{:.2}, {:.2}] at {}%: {}",
                g.statistic,
                if passed { "within" } else { "outside" },
                g.lower,
                g.upper,
                criteria.global_alpha * 100.0,
                if passed { "passed" } else { "failed" }
            ),
            None => "not possible without redundancy".into(),
        },
        criteria.w,
        criteria.alpha0,
        criteria.beta0,
        criteria.lambda0,
        criteria.vector,
        criteria.vector_alpha * 100.0
    );

    // Suspects.
    let name = |m: usize| escape(&n.marks()[m].name);
    let observation = |o: usize| {
        input
            .observations
            .get(o)
            .map(|i| (escape(&i.label), escape(&i.source)))
            .unwrap_or_else(|| (escape(&n.observations()[o].id), String::new()))
    };
    out.push_str("<section id=\"suspects\">\n<h2>Suspects</h2>\n");
    if suspects.is_empty() {
        out.push_str("<p>No test failed.</p>\n");
    } else {
        out.push_str(
            "<p>Ranked by how far each test failed. Each is for a person to judge: exclude or \
             re-weight it with a reason, or keep it.</p>\n<div class=\"scroll\"><table>\n\
             <tr><th class=\"n\">Rank</th><th>What</th><th>Source</th><th>Test</th>\
             <th class=\"n\">Statistic</th><th class=\"n\">Critical</th><th class=\"n\">Excess</th>\
             <th class=\"n\">MDB (mm)</th></tr>\n",
        );
        for (rank, s) in suspects.iter().enumerate() {
            let (what, source, mdb) = match s.subject {
                Subject::Element {
                    observation: o,
                    element,
                } => {
                    let (label, source) = observation(o);
                    let what = if n.observations()[o].elements.len() > 1 {
                        format!("{label}, element {}", element + 1)
                    } else {
                        label
                    };
                    let mdb = s
                        .component
                        .and_then(|c| a.observations()[o].elements[element].components[c].mdb);
                    (what, source, mdb)
                }
                Subject::Setup(setup) => {
                    let mark = n.setups()[setup].mark;
                    let mdb = s
                        .component
                        .and_then(|c| a.setups()[setup].components[c].mdb);
                    (
                        format!("set-up {} over {}", setup + 1, name(mark)),
                        String::new(),
                        mdb,
                    )
                }
            };
            let (test, critical) = match s.component {
                Some(c) => (format!("w {}", ["east", "north", "up"][c]), criteria.w),
                None => ("vector T".into(), criteria.vector),
            };
            let _ = writeln!(
                out,
                "<tr><td class=\"n\">{}</td><td>{what}</td><td>{source}</td><td>{test}</td>\
                 <td class=\"n\">{:.2}</td><td class=\"n\">{critical:.2}</td>\
                 <td class=\"n\">{:.2}</td><td class=\"n\">{}</td></tr>",
                rank + 1,
                s.statistic,
                s.excess,
                mdb.map(mm).unwrap_or_else(|| "–".into())
            );
        }
        out.push_str("</table></div>\n");
    }
    let edits = a.edits();
    if !edits.is_empty() {
        out.push_str(
            "<h3>Edits</h3>\n<div class=\"scroll\"><table>\n<tr><th>Observation</th>\
             <th>Action</th><th>Reason</th></tr>\n",
        );
        for e in edits {
            let action = match e.action {
                Action::Exclude => "excluded".to_owned(),
                Action::Reweight(f) => format!("covariance × {f}"),
            };
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{action}</td><td>{}</td></tr>",
                escape(&e.observation),
                escape(&e.reason)
            );
        }
        out.push_str("</table></div>\n");
    }
    out.push_str("</section>\n");

    // Coordinates.
    let mut weighted = vec![false; n.marks().len()];
    for o in n.observations() {
        for e in &o.elements {
            if let Element::Position { mark, .. } = e {
                weighted[*mark] = true;
            }
        }
    }
    let grid = MgaGrid::for_frame(n.frame());
    let _ = write!(
        out,
        "<section id=\"coordinates\">\n<h2>Coordinates</h2>\n<p>Adjusted positions in {}{epoch}, \
         with ellipsoidal heights. Standard deviations are on local east, north and up axes; \
         positional uncertainty (PU) is SP1's, at 95%: the circular horizontal measure and \
         1.96σ vertically.</p>\n<div class=\"scroll\"><table>\n<tr><th>Mark</th><th>Held</th>\
         <th>Latitude</th><th>Longitude</th><th class=\"n\">Height (m)</th>{}\
         <th class=\"n\">σE (mm)</th><th class=\"n\">σN (mm)</th><th class=\"n\">σU (mm)</th>\
         <th class=\"n\">PU H95 (mm)</th><th class=\"n\">PU V95 (mm)</th></tr>\n",
        escape(n.frame().name()),
        if grid.is_some() {
            "<th class=\"n\">Zone</th><th class=\"n\">Easting (m)</th><th class=\"n\">Northing (m)</th>"
        } else {
            ""
        }
    );
    for (i, mark) in a.marks().iter().enumerate() {
        let Ok(position) = a.position(i, input.scaling) else {
            continue;
        };
        let g = position.geodetic();
        let held = if mark.fixed {
            "fixed"
        } else if weighted[i] {
            "weighted"
        } else {
            "free"
        };
        let mga = match grid.and_then(|_| to_mga(&position, None).ok()) {
            Some((c, _)) => format!(
                "<td class=\"n\">{}</td><td class=\"n\">{:.4}</td><td class=\"n\">{:.4}</td>",
                c.zone.number(),
                c.easting_m,
                c.northing_m
            ),
            None if grid.is_some() => "<td></td><td></td><td></td>".into(),
            None => String::new(),
        };
        let (sd, pu) = match a.positional_uncertainty(i, input.scaling) {
            Ok(u) => {
                let sd = |k: usize| mm(libm::sqrt(u.enu[(k, k)].max(0.0)));
                (
                    format!(
                        "<td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>",
                        sd(0),
                        sd(1),
                        sd(2)
                    ),
                    format!(
                        "<td class=\"n\">{}</td><td class=\"n\">{}</td>",
                        mm(u.horizontal_95),
                        mm(u.vertical_95)
                    ),
                )
            }
            Err(_) => (
                "<td></td><td></td><td></td>".into(),
                "<td></td><td></td>".into(),
            ),
        };
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{held}</td><td>{}</td><td>{}</td><td class=\"n\">{:.4}</td>{mga}{sd}{pu}</tr>",
            name(i),
            dms(g.latitude_deg()),
            dms(g.longitude_deg()),
            g.ellipsoidal_height_m()
        );
    }
    out.push_str("</table></div>\n</section>\n");

    // Relative uncertainty between observed pairs.
    let mut pairs = Vec::new();
    let mut seen = BTreeSet::new();
    for o in n.observations() {
        for e in &o.elements {
            if let Element::Difference { from, to, .. } = *e {
                let key = (from.min(to), from.max(to));
                if seen.insert(key) {
                    pairs.push((from, to));
                }
            }
        }
    }
    out.push_str(
        "<section id=\"relative\">\n<h2>Relative uncertainty</h2>\n<p>SP1's relative \
         uncertainty (RU) at 95% between each pair of marks joined by an observation.</p>\n\
         <div class=\"scroll\"><table>\n<tr><th>From</th><th>To</th><th class=\"n\">Distance (m)</th>\
         <th class=\"n\">RU H95 (mm)</th><th class=\"n\">RU V95 (mm)</th><th class=\"n\">H95 (ppm)</th></tr>\n",
    );
    for (from, to) in pairs {
        let distance = (a.marks()[to].position - a.marks()[from].position).norm();
        if let Ok(u) = a.relative_uncertainty(from, to, input.scaling) {
            let _ = writeln!(
                out,
                "<tr><td>{}</td><td>{}</td><td class=\"n\">{distance:.3}</td><td class=\"n\">{}</td>\
                 <td class=\"n\">{}</td><td class=\"n\">{:.2}</td></tr>",
                name(from),
                name(to),
                mm(u.horizontal_95),
                mm(u.vertical_95),
                if distance > 0.0 {
                    u.horizontal_95 / distance * 1e6
                } else {
                    0.0
                }
            );
        }
    }
    out.push_str("</table></div>\n</section>\n");

    // Loop closures and repeat baselines.
    out.push_str("<section id=\"closures\">\n<h2>Loop closures</h2>\n");
    match input.closures {
        None => out.push_str("<p>Not computed.</p>\n"),
        Some(c) => {
            let failed_loops = c.loops.iter().filter(|l| !l.passed).count();
            let failed_repeats = c.repeats.iter().filter(|r| !r.passed).count();
            let _ = writeln!(
                out,
                "<p>{} fundamental loop{} ({failed_loops} failed) and {} repeated vector{} \
                 ({failed_repeats} failed), each tested against χ²(3) with a critical value of \
                 {:.2}, the clusters' correlations included. The worst ten of each are listed, \
                 and any others that failed.</p>",
                c.loops.len(),
                if c.loops.len() == 1 { "" } else { "s" },
                c.repeats.len(),
                if c.repeats.len() == 1 { "" } else { "s" },
                c.critical
            );
            let mut loops: Vec<_> = c.loops.iter().collect();
            loops.sort_by(|x, y| y.statistic.total_cmp(&x.statistic));
            let shown: Vec<_> = loops
                .iter()
                .enumerate()
                .filter(|(k, l)| *k < 10 || !l.passed)
                .map(|(_, l)| *l)
                .collect();
            if !shown.is_empty() {
                out.push_str(
                    "<h3>Loops</h3>\n<div class=\"scroll\"><table>\n<tr><th>Marks</th>\
                     <th class=\"n\">Length (m)</th><th class=\"n\">E (mm)</th><th class=\"n\">N (mm)</th>\
                     <th class=\"n\">U (mm)</th><th class=\"n\">ppm</th><th class=\"n\">Statistic</th>\
                     <th>Result</th></tr>\n",
                );
                for l in shown {
                    let marks: Vec<String> = l.marks.iter().map(|&m| name(m)).collect();
                    let _ = writeln!(
                        out,
                        "<tr{}><td>{}</td><td class=\"n\">{:.1}</td><td class=\"n\">{}</td>\
                         <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{:.2}</td>\
                         <td class=\"n\">{:.2}</td><td>{}</td></tr>",
                        if l.passed { "" } else { " class=\"flag\"" },
                        marks.join(" → "),
                        l.length,
                        mm(l.misclosure_enu.x),
                        mm(l.misclosure_enu.y),
                        mm(l.misclosure_enu.z),
                        l.ppm,
                        l.statistic,
                        if l.passed { "passed" } else { "failed" }
                    );
                }
                out.push_str("</table></div>\n");
            }
            let mut repeats: Vec<_> = c.repeats.iter().collect();
            repeats.sort_by(|x, y| y.statistic.total_cmp(&x.statistic));
            let shown: Vec<_> = repeats
                .iter()
                .enumerate()
                .filter(|(k, r)| *k < 10 || !r.passed)
                .map(|(_, r)| *r)
                .collect();
            if !shown.is_empty() {
                out.push_str(
                    "<h3>Repeated vectors</h3>\n<div class=\"scroll\"><table>\n<tr><th>First</th>\
                     <th>Repeat</th><th class=\"n\">Length (m)</th><th class=\"n\">E (mm)</th>\
                     <th class=\"n\">N (mm)</th><th class=\"n\">U (mm)</th><th class=\"n\">ppm</th>\
                     <th class=\"n\">Statistic</th><th>Result</th></tr>\n",
                );
                for r in shown {
                    let _ = writeln!(
                        out,
                        "<tr{}><td>{}</td><td>{}</td><td class=\"n\">{:.1}</td><td class=\"n\">{}</td>\
                         <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{:.2}</td>\
                         <td class=\"n\">{:.2}</td><td>{}</td></tr>",
                        if r.passed { "" } else { " class=\"flag\"" },
                        observation(r.first.0).1,
                        observation(r.repeat.0).1,
                        r.length,
                        mm(r.difference_enu.x),
                        mm(r.difference_enu.y),
                        mm(r.difference_enu.z),
                        r.ppm,
                        r.statistic,
                        if r.passed { "passed" } else { "failed" }
                    );
                }
                out.push_str("</table></div>\n");
            }
        }
    }
    out.push_str("</section>\n");

    // Observations.
    out.push_str(
        "<section id=\"observations\">\n<h2>Observations</h2>\n<p>Residuals (adjusted minus \
         observed) and w-tests on local east, north and up axes at each vector's first mark, the \
         vector test T, and the smallest redundancy number. Highlighted rows failed a test; \
         excluded observations show their misclosure against the network adjusted without \
         them.</p>\n<div class=\"scroll\"><table>\n<tr><th>Observation</th><th>Source</th>\
         <th>From</th><th>To</th><th class=\"n\">vE (mm)</th><th class=\"n\">vN (mm)</th>\
         <th class=\"n\">vU (mm)</th><th class=\"n\">wE</th><th class=\"n\">wN</th>\
         <th class=\"n\">wU</th><th class=\"n\">T</th><th class=\"n\">Min r</th><th>Used</th></tr>\n",
    );
    for (o, (obs, result)) in n.observations().iter().zip(a.observations()).enumerate() {
        let (label, source) = observation(o);
        for (k, (e, r)) in obs.elements.iter().zip(&result.elements).enumerate() {
            let (from, to) = match *e {
                Element::Difference { from, to, .. } => (name(from), name(to)),
                Element::Position { mark, .. } => (name(mark), "(position)".into()),
            };
            let flagged = r
                .components
                .iter()
                .any(|c| c.w.is_some_and(|w| w.abs() > criteria.w))
                || r.vector_test.is_some_and(|t| t > criteria.vector);
            let what = if obs.elements.len() > 1 {
                format!("{label}, element {}", k + 1)
            } else {
                label.clone()
            };
            let redundancy = r
                .components
                .iter()
                .map(|c| c.redundancy)
                .fold(f64::INFINITY, f64::min);
            let _ = writeln!(
                out,
                "<tr{}><td>{what}</td><td>{source}</td><td>{from}</td><td>{to}</td>\
                 <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>\
                 <td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td>\
                 <td class=\"n\">{}</td><td class=\"n\">{redundancy:.2}</td><td>{}</td></tr>",
                if flagged { " class=\"flag\"" } else { "" },
                mm(r.components[0].residual),
                mm(r.components[1].residual),
                mm(r.components[2].residual),
                optional(r.components[0].w, 2),
                optional(r.components[1].w, 2),
                optional(r.components[2].w, 2),
                optional(r.vector_test, 2),
                if result.used { "yes" } else { "excluded" }
            );
        }
    }
    out.push_str("</table></div>\n</section>\n");

    // Provenance.
    out.push_str(
        "<section id=\"provenance\">\n<h2>Provenance</h2>\n<h3>Input files</h3>\n\
         <div class=\"scroll\"><table>\n<tr><th>File</th><th>Read as</th><th class=\"n\">Bytes</th>\
         <th>SHA-256</th></tr>\n",
    );
    for f in &input.files {
        let _ = writeln!(
            out,
            "<tr><td>{}</td><td>{}</td><td class=\"n\">{}</td><td><code>{}</code></td></tr>",
            escape(&f.name),
            escape(&f.format),
            f.bytes,
            escape(&f.sha256)
        );
    }
    out.push_str("</table></div>\n");
    let list = |out: &mut String, heading: &str, items: &[String], empty: &str| {
        let _ = writeln!(out, "<h3>{heading}</h3>");
        if items.is_empty() {
            let _ = writeln!(out, "<p class=\"note\">{empty}</p>");
        } else {
            out.push_str("<ul>\n");
            for item in items {
                let _ = writeln!(out, "<li>{}</li>", escape(item));
            }
            out.push_str("</ul>\n");
        }
    };
    list(
        &mut out,
        "Transformations",
        &input.transformations,
        "None: every measurement was already in the network's frame and epoch.",
    );
    list(&mut out, "Notes", &input.notes, "None.");
    if !input.settings.is_empty() {
        out.push_str("<h3>Settings</h3>\n<dl>\n");
        for (k, v) in &input.settings {
            let _ = writeln!(out, "<dt>{}</dt><dd>{}</dd>", escape(k), escape(v));
        }
        out.push_str("</dl>\n");
    }
    out.push_str("</section>\n</body>\n</html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn writes_angles_in_degrees_minutes_and_seconds() {
        assert_eq!(dms(-36.563403753), "−36°33′48.25351″");
        assert_eq!(dms(145.0), "145°00′00.00000″");
        // 59.999999 seconds rounds into the next minute.
        assert_eq!(
            dms(10.0 + 59.0 / 60.0 + 59.999_999_9 / 3600.0),
            "11°00′00.00000″"
        );
    }

    /// The web app shows reports in the page under a Content Security Policy
    /// that allows only its own styles, so it names this style by its hash.
    #[test]
    fn the_web_apps_security_policy_allows_the_style() {
        use sha2::Digest;
        const HEADERS: &str = include_str!("../../../apps/web/ui/public/_headers");
        let digest = sha2::Sha256::digest(STYLE.as_bytes());
        let source = format!("'sha256-{}'", base64(&digest));
        assert!(
            HEADERS.contains(&source),
            "the report's style changed: in apps/web/ui/public/_headers, style-src needs {source}"
        );
    }

    fn base64(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                if i <= chunk.len() {
                    out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    #[test]
    fn escapes_html() {
        assert_eq!(
            escape("<a href='x'>&\"</a>"),
            "&lt;a href=&#39;x&#39;&gt;&amp;&quot;&lt;/a&gt;"
        );
    }
}
