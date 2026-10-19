# Contributing

Thanks for your interest. The project is at an early stage: the plan is written ([docs/PLAN.md](docs/PLAN.md)), and the foundations are being built milestone by milestone ([docs/ROADMAP.md](docs/ROADMAP.md)).

## Ways to help right now

- **Review the plan.** Surveyors, geodesists and GNSS engineers: open an issue if something in the plan is wrong, missing or impractical.
- **Offer data for the benchmark.** Static GNSS observations on marks with known coordinates are especially valuable from low-cost receivers. Use the "Offer a dataset" issue form. See [docs/VALIDATION.md](docs/VALIDATION.md) for what's useful.
- **Code.** Pick up an issue for the current milestone, or open one to discuss what you'd like to work on before you start. Large changes need a discussion first.

## Development setup

| Tool | Why | How |
|---|---|---|
| Rust | The core, CLI and web core | Install [rustup](https://rustup.rs). It reads `rust-toolchain.toml` and installs the pinned toolchain and the `wasm32-unknown-unknown` target. |
| wasm-bindgen-cli | Runs the WebAssembly tests in a browser | `cargo install wasm-bindgen-cli --version <the version in Cargo.lock> --locked` |
| Chrome and chromedriver | The browser for those tests | Matching versions. Set `CHROMEDRIVER` to the chromedriver binary. |
| Node.js 22 | The web app | From [nodejs.org](https://nodejs.org) |
| wasi-sdk 34 | Only to rebuild engine modules | See [engine-modules/rtklib-ex](engine-modules/rtklib-ex/README.md) |

## Checks to run before a pull request

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# The same tests, compiled to WebAssembly and run in headless Chrome:
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
  cargo test --target wasm32-unknown-unknown \
  --workspace --exclude gnss-engine-host --exclude gnss-cli --exclude gnss-relay
```

For the web app, in `apps/web/ui` (needs the engine module built, see [engine-modules/rtklib-ex](engine-modules/rtklib-ex/README.md)):

```bash
npm ci
npm run licences && npm run build && npm run check
npm run test:e2e -- <RTKLIB-EX checkout>/test/data/rinex   # engine in Chromium vs golden output
npm run test:app                                           # storage, hashing and offline use
```

CI runs the same checks, with the Rust ones on Windows, macOS and Linux, plus cargo-deny.

## Conventions

- **Code:** follow [docs/CONVENTIONS.md](docs/CONVENTIONS.md). The short version: SI units, every coordinate carries its frame and epoch, no panics on user data, and the core never touches the file system, network or clock directly.
- **Design changes:** update [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) in the same pull request. Record significant decisions in [docs/DECISIONS.md](docs/DECISIONS.md), using a full ADR in `docs/adr/` when a decision needs more than a few lines.
- **Documentation:** written in Australian English.
- **Pull requests:** keep each one focused on a single change, with tests. Explain the why in the description.

## Licensing

- **Your contributions** are licensed under the [Apache License 2.0](LICENSE), the project's licence.
- **Dependencies** must be permissively licensed. GPL, AGPL and LGPL code can't be linked into the project (decision D-005). cargo-deny enforces this in CI. If you need a copyleft tool, raise it in an issue first.

## Developer Certificate of Origin

Every commit must be signed off, certifying the [Developer Certificate of Origin](https://developercertificate.org/): that you wrote the change, or otherwise have the right to submit it under the project's licence. Sign off by committing with `-s`:

```bash
git commit -s -m "Explain what this change does"
```

This adds a `Signed-off-by: Your Name <you@example.com>` line, using your git name and email. CI checks sign-off on pull requests from forks. To fix an unsigned commit, run `git commit --amend -s` (or `git rebase --signoff` for several commits) and force-push your branch.

## Data and sensitive locations

Don't attach observation data, coordinates or project files from security-sensitive locations to issues or pull requests. Benchmark datasets are published under CC BY 4.0, so only offer data you have the right to share on those terms.

## Conduct

Everyone taking part is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).
