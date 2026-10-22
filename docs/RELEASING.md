# Releasing

A release is a tag. Pushing `v<version>` runs `.github/workflows/release.yml`, which builds, checks and publishes everything. Nothing is built or uploaded by hand.

## Making a release

1. **Set the version** in two places, in one commit on main:
   - `Cargo.toml`: `version` under `[workspace.package]` (every crate takes it from there)
   - `apps/web/ui/package.json`: `version`

   Run `cargo check` so `Cargo.lock` picks it up, and `npm install --package-lock-only` in `apps/web/ui` for `package-lock.json`.
2. **Wait for main's CI to pass**, and check the development site at `https://main.<project>.pages.dev`.
3. **Tag the commit and push the tag:**

   ```sh
   git tag -a v0.1.0 -m "v0.1.0"
   git push origin v0.1.0
   ```

A version with a hyphen (`0.2.0-rc.1`) is a prerelease: it's published on GitHub, marked as a prerelease, but not deployed to the site.

**Without a git client** (from a phone, say), do step 3 from the *Actions* tab instead: *Release → Run workflow*, on `main`, with *Tag main as v<version> and publish the release* ticked. The workflow checks that the version isn't tagged yet, builds and checks everything as for a pushed tag, then tags the commit it built and publishes it.

To try the whole workflow without publishing, run it by hand with that box left unticked, on any branch. It builds and checks everything, skips the version-to-tag check, and publishes nothing.

## What the workflow does

1. **Checks the version.** The tag, the crates and the web app must all say the same version, or nothing runs.
2. **Runs all of CI** (`ci.yml`), exactly as on a pull request.
3. **Builds the CLI** for each platform, packaged with `LICENSE`, `NOTICE` and `README.md`. Each binary that can run on its build machine is run, and must report the release's version.
4. **Packages the web bundle**: the site CI just built and tested, zipped.
5. **Publishes a GitHub release** with generated notes, and the assets below.
6. **Deploys the site** (not for prereleases): every release's web bundle is downloaded from GitHub and checked against that release's `SHA256SUMS`, then assembled by `apps/web/ui/scripts/assemble-site.mjs` with the latest at the root and each release under `/v/<version>/`, and uploaded to the Pages project's production branch ([DEPLOYING.md](DEPLOYING.md)). Older versions are rebuilt from their published bundles, never from source, so they don't change.

## What's published

| Asset | What it is |
|---|---|
| `gnss-<version>-x86_64-unknown-linux-musl.tar.gz` | The CLI for Linux (x86-64), statically linked |
| `gnss-<version>-x86_64-pc-windows-msvc.zip` | The CLI for Windows (x86-64) |
| `gnss-<version>-aarch64-apple-darwin.tar.gz` | The CLI for macOS on Apple silicon |
| `gnss-<version>-x86_64-apple-darwin.tar.gz` | The CLI for macOS on Intel |
| `gnss-web-<version>.zip` | The web app as static files, for air-gapped use or an intranet server |
| `gnss-<version>-cli.cdx.json` | SBOM of the CLI's Rust crates (CycloneDX) |
| `gnss-web-<version>-core.cdx.json` | SBOM of the web app's Rust crates, built for WebAssembly |
| `gnss-web-<version>-npm.cdx.json` | SBOM of the web app's JavaScript packages |
| `SHA256SUMS` | SHA-256 of every file above |

Every asset also has a build provenance attestation, signed with Sigstore through GitHub. It records that the file was built by this repository's release workflow from the tagged commit. To check a download:

```sh
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify gnss-web-0.1.0.zip --repo chrislandvogt/GNSS
```

The CLI binaries aren't code-signed yet (ARCHITECTURE.md section 18), so Windows SmartScreen and macOS Gatekeeper warn on first run. On macOS, `xattr -d com.apple.quarantine gnss` clears it after checking the attestation.

## If a release goes wrong

- **The workflow fails before *Publish*:** nothing was published. Fix it on main, delete the tag (`git push origin :v0.1.0`, then `git tag -d v0.1.0`), and tag again.
- **It fails after *Publish*, in *Deploy*:** the release is on GitHub but the site isn't updated. Fix the cause (usually the Cloudflare secrets) and re-run the failed job.
- **A published release is broken:** don't replace its files; `/v/<version>/` is meant to stay as it is. Release a new patch version.
