# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately, through GitHub's private vulnerability reporting: go to the repository's **Security** tab and choose **Report a vulnerability**. Don't open a public issue.

Include what you found, how to reproduce it, and what an attacker could do with it. You'll get an acknowledgement, and we'll agree a disclosure date with you once a fix is ready.

## Supported versions

Nothing has been released yet. Once releases begin, security fixes go into the latest release. Each release stays available at its own URL for reproducibility, so the release notes say which older releases are affected.

## What's in scope

These are the areas we most want to hear about (see the threat model in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#19-security-and-privacy)):

- **Parsers of untrusted input:** RINEX, vector and control formats, field logs, products and project files.
- **Engine modules:** anything that lets a crafted input escape the WebAssembly sandbox or affect other jobs.
- **The web app:** Content Security Policy bypasses, cross-origin access to project data, service worker cache poisoning.
- **`gnss serve` and the relay:** requests from other origins, forwarding to undeclared hosts, or leaks of which files a user requested.
- **The supply chain:** build and release integrity, dependency or engine module tampering.
- **Privacy:** any way project data or coordinates leave the user's machine without them configuring it.
