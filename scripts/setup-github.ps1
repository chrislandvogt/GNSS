# Create the roadmap milestones (M0 to M13) and the issue labels on GitHub.
#
# A one-off setup task from docs/ROADMAP.md (M0). Safe to run again: labels
# are updated in place, and milestones that already exist are skipped.
#
# Requires the GitHub CLI (https://cli.github.com), logged in with
# `gh auth login`, with write access to the repository.
#
# Usage, from PowerShell:
#   ./scripts/setup-github.ps1                      # chrislandvogt/GNSS
#   ./scripts/setup-github.ps1 -Repo owner/name

param(
    [string]$Repo = "chrislandvogt/GNSS"
)

$ErrorActionPreference = "Stop"

$milestones = @(
    @{ title = "M0 Bootstrap"; description = "A repo people can build, test and contribute to, and evidence that the browser-first architecture works (D-023)." }
    @{ title = "M1 Datum and geodesy core"; description = "Frames, epochs and transformations that are correct by construction." }
    @{ title = "M2 Adjustment engine"; description = "A rigorous, explainable 3D least squares adjustment of GNSS observations." }
    @{ title = "M3 Formats and CLI"; description = "Get vectors in and results out, without a GUI." }
    @{ title = "M4 v0.1 release"; description = "The second-opinion tool in real users' hands." }
    @{ title = "M5 Project model and pipeline"; description = "An occupation-first project folder and a cached, reproducible build graph." }
    @{ title = "M6 RINEX ingest and QC"; description = "Trustworthy observation data and the metadata to check it." }
    @{ title = "M7 Data fetch"; description = "No more data chasing." }
    @{ title = "M8 RTKLIB-EX baseline processing"; description = "Reliable static baselines with honest quality information." }
    @{ title = "M9 Blunder checks"; description = "Catch the errors that actually wreck surveys." }
    @{ title = "M10 Engine benchmark v1"; description = "Public evidence of how well the engine works, and where it stops working." }
    @{ title = "M11 Web app"; description = "The full workflow in an app surveyors want to use." }
    @{ title = "M12 Reporting and standards"; description = "Reports that clients and agencies accept." }
    @{ title = "M13 v1.0 hardening"; description = "Production quality." }
)

$labels = @(
    @{ name = "type/bug"; color = "d73a4a"; description = "Something gives a wrong answer, fails or crashes" }
    @{ name = "type/feature"; color = "a2eeef"; description = "Something the tool should do, or do differently" }
    @{ name = "type/dataset"; color = "0e8a16"; description = "An offer of GNSS data for the benchmark" }
    @{ name = "type/docs"; color = "0075ca"; description = "Documentation" }
    @{ name = "type/chore"; color = "c5def5"; description = "Build, CI, dependencies and housekeeping" }
    @{ name = "area/datum"; color = "5319e7"; description = "Frames, epochs, transformations, geoid, projections" }
    @{ name = "area/adjust"; color = "5319e7"; description = "Least squares adjustment, testing, reliability, uncertainty" }
    @{ name = "area/formats"; color = "5319e7"; description = "Import and export formats" }
    @{ name = "area/rinex"; color = "5319e7"; description = "RINEX ingest and quality control" }
    @{ name = "area/fetch"; color = "5319e7"; description = "Data sources, downloads, cache and data packs" }
    @{ name = "area/engine"; color = "5319e7"; description = "Baseline engines and engine modules" }
    @{ name = "area/checks"; color = "5319e7"; description = "Blunder detection" }
    @{ name = "area/pipeline"; color = "5319e7"; description = "Build graph, cache and provenance" }
    @{ name = "area/web"; color = "1d76db"; description = "The web app" }
    @{ name = "area/cli"; color = "1d76db"; description = "The command-line interface" }
    @{ name = "area/validation"; color = "fbca04"; description = "Benchmark, oracles and release gates" }
    @{ name = "area/standards"; color = "fbca04"; description = "SP1 and jurisdiction packs" }
    @{ name = "good first issue"; color = "7057ff"; description = "A good way into the project" }
)

foreach ($label in $labels) {
    gh label create $label.name --repo $Repo --color $label.color --description $label.description --force | Out-Null
    Write-Host "label     $($label.name)"
}

$existing = gh api "repos/$Repo/milestones?state=all&per_page=100" --jq ".[].title"
foreach ($milestone in $milestones) {
    if ($existing -contains $milestone.title) {
        Write-Host "milestone $($milestone.title) (already exists)"
        continue
    }
    gh api "repos/$Repo/milestones" -f "title=$($milestone.title)" -f "description=$($milestone.description)" | Out-Null
    Write-Host "milestone $($milestone.title)"
}

Write-Host "`nDone. To add a project board, create one at https://github.com/users/<you>/projects and add the repository."
