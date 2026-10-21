# Deploying the web app

The web app is static files, served by Cloudflare Pages (ARCHITECTURE.md section 18). Cloudflare doesn't build it. CI builds the site, tests it in three browsers, and then the `deploy` job uploads exactly that bundle with Wrangler. So what's served is what was tested, byte for byte, and the build needs nothing beyond what CI already has (Rust, wasm-bindgen, the WASI engine module).

| What | Where | When |
|---|---|---|
| The development version: main as it stands | `https://main.<project>.pages.dev` | Every push to main, once all of CI has passed |
| Releases (from M4) | `https://<project>.pages.dev/` (the latest release) and `/v/<version>/` (every release, never changed) | Each release tag |

The production address stays empty until the first release.

## Setting it up

Once, by a maintainer with access to the Cloudflare account and the repository's settings:

1. **An API token.** In the Cloudflare dashboard, go to *My Profile → API Tokens → Create Token → Create Custom Token*. Give it one permission, *Account → Cloudflare Pages → Edit*, for this account only.
2. **The account ID.** It's shown on the dashboard's *Workers & Pages* page, and in the address bar after `dash.cloudflare.com/`.
3. **GitHub secrets.** In the repository's *Settings → Secrets and variables → Actions*, add two repository secrets:
   - `CLOUDFLARE_API_TOKEN`: the token
   - `CLOUDFLARE_ACCOUNT_ID`: the account ID
4. **Optionally, the project's name.** Add a repository variable `CLOUDFLARE_PAGES_PROJECT` to use another name. The default is `gnss`.

The next push to main deploys. If the Pages project doesn't exist, the first deploy creates it, with `production` as its production branch, so releases and main never overwrite each other. Cloudflare picks the address: `gnss.pages.dev` if it's free, or a variant if not. The deploy job's log shows it.

Until the secrets are set, the deploy job does nothing and says so; CI stays green.

**Don't connect the Pages project to Git.** A Git-connected project rebuilds the site from source on Cloudflare, which needs the whole Rust and WASI toolchain there, and serves a build CI never tested. If one already exists under the same name, delete it, or set `CLOUDFLARE_PAGES_PROJECT` to a new name.

## What the host must do

- **Apply `_headers`.** The Content Security Policy, `nosniff`, `no-referrer` and `same-origin` opener policy come from `apps/web/ui/public/_headers`, which Pages applies. The policy allows one inline style by its hash: the HTML report's (the report crate's tests check the hash).
- **Serve `sw.js` uncached**, as `_headers` asks, so a new version is found on the next visit.
- **HTTPS.** Pages always serves it; the app needs a secure context for workers and browser storage.

## A custom domain

Attach it to the Pages project under *Custom domains*. It serves the production branch, which releases fill.
