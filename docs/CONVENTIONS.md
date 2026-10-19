# Conventions

These rules keep the code correct by construction and the same in every build. They expand on [ARCHITECTURE.md section 4](ARCHITECTURE.md#4-conventions). When code and this document disagree, fix one of them in the same pull request.

## 1. Units

- **SI everywhere inside the core.** Lengths in metres, times in seconds, angles in radians, variances in m².
- **Other units only at the edges.** Degrees, millimetres, ppm, DMS and feet appear only when parsing input or formatting output, and the conversion happens in the parser or formatter, nowhere else.
- **Names carry units when the type doesn't.** `height_m`, `interval_s`, `elev_mask_rad`. A bare `f64` named `height` is a review comment.
- **Prefer types over suffixes for core quantities.** Positions, vectors, epochs and covariances get their own types (below) so that mixing them up doesn't compile.

## 2. Coordinates, frames and epochs

- **ECEF is the working representation.** Geodetic (latitude, longitude, ellipsoidal height) and projected coordinates (for example MGA2020) are views computed from ECEF, not separate sources of truth.
- **Every position carries its frame and epoch.** `Position { frame, epoch, xyz, cov }`. There's no API that takes bare XYZ for a position in the world.
- **No implicit transformations.** Combining positions across frames or epochs needs an explicit transformation from `datum`, which records the path it took. Comparing a GDA2020 coordinate with an ITRF2014 one without transforming is a type error where the types can express it, and a runtime error where they can't.
- **Static and dynamic frames behave differently.** Static frames (GDA2020, GDA94) have a fixed reference epoch. Dynamic frames (ITRF2020, IGS20, ITRF2014, ATRF2014) need the epoch of the coordinate.
- **Vectors are ECEF differences**, with the frame and epoch of the orbits used to compute them.
- **Signs and order.** A baseline from A to B is `B − A`. Local frames are east, north, up (ENU), in that order.

## 3. Heights

- **Heights are typed.** `Ellipsoidal`, `Ahd`, `Avws` and so on, each recording the model used (for example AUSGeoid2020). There's no untyped "height".
- **Antenna heights say what they measure.** The method (vertical or slant), the measure-up point, and the value as measured. The reduction to the antenna reference point happens in one place, using the antenna geometry catalogue.

## 4. Time

- **Observation times are hifitime epochs in GPST.** Conversions to and from UTC handle leap seconds through hifitime, never by hand.
- **Coordinate epochs are decimal years** (`CoordinateEpoch`), computed by one documented function, `CoordinateEpoch::from_time`. No other code converts dates to decimal years. The convention is the calendar year plus the elapsed fraction of it in GPST, so 1 January 2018 is 2018.0.
- **Time spans are half-open**: `[start, end)`.
- **The core never reads the clock.** Run timestamps in provenance come from the host (section 9).

## 5. Covariances and uncertainty

- **Store full covariance matrices in m².** Never keep only standard deviations where correlations exist, and never store correlation coefficients instead of covariances.
- **Uncertainty names say what they are.** `sigma` is one standard deviation. Values at 95% confidence are named for it (`pu_95_h_m`). The confidence level is never implied.
- **Engine output is decoded exactly.** For example, RTKLIB's off-diagonal terms are signed square roots and must be squared back with their sign (ARCHITECTURE.md section 9.3).

## 6. Errors and panics

- **Libraries return errors; they don't panic on user data.** Use `thiserror` for error types in library crates, and `anyhow` only in binaries.
- **`unwrap` and `expect` are linted** (`clippy::unwrap_used`, `clippy::expect_used`). Tests may use them. Elsewhere, an `expect` needs a comment explaining why it can't fail, and an `#[allow]` on the narrowest scope.
- **Errors explain themselves.** Name the file, line, mark, occupation or observation involved, and say what was expected. "Invalid input" is not an error message.
- **Parsers are fuzzed.** Every parser of external data gets a fuzz target in the same pull request that adds it.

## 7. Logging and provenance

- **Structured logging with `tracing`.** Logs help developers. They aren't a record of results.
- **Provenance is the record.** Anything that affects a result (inputs, settings, versions, engine module hashes, user decisions such as excluding an observation) goes into provenance, not just the log.
- **No telemetry.** Nothing is sent anywhere unless the user configures a destination (D-015).

## 8. Numerics and determinism

- **f64 throughout.** No f32 in computations.
- **One maths library everywhere (D-024).** Use the `libm` crate for transcendental functions: `libm::sin(x)`, not `x.sin()`. The `f64` methods call each platform's own maths library, and those round differently. Clippy rejects them (`clippy.toml`). Arithmetic, `sqrt`, `mul_add`, `abs` and rounding are exact everywhere and are fine.
- **Deterministic iteration order** in anything that feeds a numeric result: `BTreeMap`, `BTreeSet`, `IndexMap` or sorted vectors, never `HashMap` or `HashSet` iteration.
- **Deterministic parallelism.** Parallel code mustn't change the order of floating-point operations. Split work into chunks that don't depend on the number of threads, and combine the results in a fixed order.
- **Tolerances are named constants**, defined centrally per crate with a comment saying where the value comes from.
- **No relaxed SIMD and no fast-math.** Both make results depend on the hardware.
- **What "identical" means:**
  - The CLI on every platform and the web app in every browser give bit-identical outputs for the same inputs.
  - Engine modules are the same binary everywhere, so they give identical outputs everywhere.
  - Golden numeric tests compare exactly, and CI runs them natively and in the browser.
- **Why it matters.** The M0 spike found that RTKLIB-EX builds whose maths libraries differ only in last-bit rounding make different ambiguity decisions on kinematic data, and end up metres apart ([report](spikes/m0-engine.md)). Anything with a threshold (outlier tests, convergence checks, ambiguity resolution) can do the same.

## 9. Host access

The core never touches the file system, network, clock, randomness or threads directly. The host provides them through interfaces:

| Need | Interface | Natively | In the browser |
|---|---|---|---|
| Files | `store` | `std::fs` | File System Access API, OPFS |
| Network | the fetch client | reqwest | the browser's `fetch` |
| Running jobs | the pipeline executor | tokio and rayon | web workers |
| Engine modules | `ModuleHost` | wasmtime | engine workers |
| Time | passed in by the host | system clock | `Date.now()` |

This is what lets one core build for both targets. It also makes the core easy to test, because tests supply fake hosts.

## 10. Code that builds for both targets

- **Every crate except `engine-host`, `apps/cli` and `apps/relay` builds for `wasm32-unknown-unknown`.** CI checks this.
- **Platform-specific code is rare and isolated**, behind `cfg(target_arch = "wasm32")`, in the crate that owns that host interface.
- **The same tests run on both builds.** Write ordinary `#[test]` functions, and add the WebAssembly test attribute alongside, so CI runs them natively and in headless Chrome:

  ```rust
  #[cfg(test)]
  mod tests {
      #[cfg(target_arch = "wasm32")]
      use wasm_bindgen_test::wasm_bindgen_test as test;
      #[cfg(target_arch = "wasm32")]
      wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

      #[test]
      fn works_the_same_everywhere() { /* ... */ }
  }
  ```

  The crate needs `wasm-bindgen-test` as a dev-dependency for `cfg(target_arch = "wasm32")`.

## 11. Naming

- **Crate directories are short** (`crates/adjust`); package names carry the project prefix (`gnss-adjust`) until the project is named (D-014).
- **Use survey and geodesy terms as surveyors use them:** occupation, mark, session, baseline, misclosure, PU and RU. Don't invent synonyms.
- **Abbreviations** follow the field: `ecef`, `enu`, `pco`, `pcv`, `ztd`, `mdb`, `vce`. Spell out anything a surveyor wouldn't recognise.

## 12. Testing

- **Every public function has tests**, close to the code.
- **Maths is tested against something independent:** published examples, an oracle (DynAdjust, GeodePy, PROJ), or a property (round trips, invariances). A test that only checks our code against its own earlier output is a regression test, and is named as one.
- **Golden files** are reviewed when created, stored next to the test, and updated only deliberately.
- **Benchmark data isn't committed.** Manifests and fetch scripts are (VALIDATION.md).

## 13. Dependencies

- **Permissive licences only.** No GPL, AGPL or LGPL code linked into the project (D-005), enforced by cargo-deny. The same applies to the web app's JavaScript packages.
- **Few and well-maintained.** Each new dependency is justified in its pull request.
- **Wrapped at the boundary.** External crates that might be replaced (the RINEX parser, the linear algebra library) sit behind our own types.
- **Everything is pinned:** `Cargo.lock` is committed, and so is the web app's lockfile. Toolchains and engine module builds are pinned too.

## 14. Documentation and comments

- **Documentation is written in Australian English**, in plain, direct sentences.
- **Every public item has a doc comment** (`missing_docs` is on). Say what it is and any units, frames or invariants. Don't restate the signature.
- **Comments explain why**, not what. Cite the source for formulas and parameters (for example "SP1 v2.2 section 4.2" or "GDA2020 Technical Manual, table 3.2").
- **Design changes update ARCHITECTURE.md** in the same pull request. Decisions go in DECISIONS.md.

## 15. Commits and pull requests

- **One change per pull request**, with tests and docs.
- **Commit messages** explain what changed and why, in the imperative ("Add the ENU rotation"), with a body when the why isn't obvious.
- **Sign off every commit** (`git commit -s`, see CONTRIBUTING.md).
