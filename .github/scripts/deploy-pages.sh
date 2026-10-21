#!/usr/bin/env bash
# Upload a built site to a branch of the Cloudflare Pages project, creating
# the project on first use with "production" as its production branch
# (docs/DEPLOYING.md). Used by CI for main and by the release workflow.
#
# Usage: deploy-pages.sh <site-dir> <branch>
# Needs CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID; PROJECT defaults to gnss.
set -euo pipefail

site="$1"
branch="$2"
project="${PROJECT:-gnss}"
wrangler="wrangler@4.148.0"

api="https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID/pages/projects/$project"
status="$(curl -sS -o /dev/null -w '%{http_code}' -H "Authorization: Bearer $CLOUDFLARE_API_TOKEN" "$api")"
case "$status" in
  200) ;;
  404) npx --yes "$wrangler" pages project create "$project" --production-branch production ;;
  *)
    echo "::error::Cloudflare answered $status for the Pages project $project; check the token's permissions (docs/DEPLOYING.md)."
    exit 1
    ;;
esac
npx --yes "$wrangler" pages deploy "$site" --project-name "$project" --branch "$branch" \
  --commit-hash "$GITHUB_SHA" --commit-dirty=false
