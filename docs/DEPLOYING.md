# Deploying the web app

The web app is static files, served by Cloudflare Pages (ARCHITECTURE.md section 18). Cloudflare doesn't build it. CI builds the site, tests it in three browsers, and then the `deploy` job uploads exactly that bundle with Wrangler. So what's served is what was tested, byte for byte, and the build needs nothing beyond what CI already has (Rust, wasm-bindgen, the WASI engine module).

| What | Where | When |
|---|---|---|
| The development version: main as it stands | `https://main.<project>.pages.dev` | Every push to main, once all of CI has passed |
| Releases | `https://<project>.pages.dev/` (the latest release) and `/v/<version>/` (every release, never changed) | Each release tag, by the release workflow ([RELEASING.md](RELEASING.md)) |

The production address stays empty until the first release.

## Setting it up

Once, by a maintainer with access to the Cloudflare account and the repository's settings:

1. **An API token.** In the Cloudflare dashboard, go to *My Profile → API Tokens → Create Token → Create Custom Token*. Give it one permission, *Account → Cloudflare Pages → Edit*, for this account only.
2. **The account ID.** It's shown on the dashboard's *Workers & Pages* page, and in the address bar after `dash.cloudflare.com/`.
3. **GitHub secrets.** In the repository's *Settings → Environments*, create an environment named `Env` (the deploy jobs name it), and add two environment secrets:
   - `CLOUDFLARE_API_TOKEN`: the token
   - `CLOUDFLARE_ACCOUNT_ID`: the account ID

   Under *Deployment branches and tags*, allow only the `main` branch and tags matching `v*`, so no other branch or pull request can use the token. Deploys then show in the repository's *Deployments* list.
4. **Optionally, the project's name.** Add a repository variable `CLOUDFLARE_PAGES_PROJECT` to use another name. The default is `gnss`.

The next push to main deploys. If the Pages project doesn't exist, the first deploy creates it, with `production` as its production branch, so releases and main never overwrite each other. Cloudflare picks the address: `gnss.pages.dev` if it's free, or a variant if not. The deploy job's log shows it.

Until the secrets are set, main's deploy job does nothing and says so; CI stays green. A release fails its deploy job instead, since a release that isn't deployed should be noticed; the GitHub release itself is still published.

**Don't connect the Pages project to Git.** A Git-connected project rebuilds the site from source on Cloudflare, which needs the whole Rust and WASI toolchain there, and serves a build CI never tested. If one already exists under the same name, delete it, or set `CLOUDFLARE_PAGES_PROJECT` to a new name.

## What the host must do

- **Apply `_headers`.** The Content Security Policy, `nosniff`, `no-referrer` and `same-origin` opener policy come from `apps/web/ui/public/_headers`, which Pages applies. The policy allows one inline style by its hash: the HTML report's (the report crate's tests check the hash).
- **Serve `sw.js` uncached**, as `_headers` asks, so a new version is found on the next visit.
- **One `_headers` for every version.** Pages reads only the site root's `_headers`, so the assembled site's allows every release's report style (by hash) and serves each `/v/<version>/sw.js` uncached. Each version's service worker names its caches by its scope, so versions on one origin don't clear each other's offline copies.
- **HTTPS.** Pages always serves it; the app needs a secure context for workers and browser storage.

## A custom domain

Attach it to the Pages project under *Custom domains*. It serves the production branch, which releases fill.

## The relay

The web app can download GA's CORS data itself, but some sources (CODE's precise products, the IGS's ANTEX file) don't send the cross-origin headers a browser needs. For those it can use a relay, `gnss-relay`, which agencies or users can run themselves:

```bash
cargo build --release -p gnss-relay
./target/release/gnss-relay --listen 127.0.0.1:8787                      # this machine only
./target/release/gnss-relay --listen 0.0.0.0:8787 --allow-origin https://gnss.example.org
```

- It forwards GET requests only to HTTPS URLs on the hosts the jurisdiction pack declares (`--pack`, default the built-in Australian pack), and follows redirects only to those hosts.
- It keeps no logs or copies, and prints only its start-up line. It still sees which files are requested, which hints at where and when a survey happened, so run it where you'd trust that to be seen.
- It uses the proxy in `HTTPS_PROXY` and the like for its own requests, and trusts the platform's certificates.
- `--allow-origin` limits which web app may use it from a browser; the default, `*`, suits a relay on your own machine.
- Behind a TLS-terminating reverse proxy if it's reachable beyond the machine: it speaks plain HTTP.
