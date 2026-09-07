---
status: accepted
date: 2026-09-06
---

# Embed an API-driven operator UI

Shaula serves a React operator UI through its existing Axum listener. Vite 8 uses
Oxc for TypeScript/JSX transforms and minification. Tailwind and official shadcn/ui
Radix components downloaded with the CLI provide the UI surface. Frontend sources, tests and the npm lockfile
live under `web/`.

The HTTP crate runs the installed Vite CLI at build time and embeds its output
using `rust-embed`, including in debug builds. Build hosts require Node and npm
dependencies; deployment retains the single Rust binary model. Cargo never runs
an implicit npm install. Frontend build failure prevents stale asset publication.

As amended by [ADR-0013](0013-require-openid-connect-for-all-http-access.md),
the UI shell and every embedded asset require a valid OIDC-derived session.
Resource data and mutations use the same authenticated origin. The daemon owns
the mandatory OIDC login/session flow; the session read exposes actor name/scopes
and a CSRF response header, without granting permissions. UI documents redirect
unauthenticated users to login; assets and APIs return 401. All responses are
private/no-store. The only anonymous routes initiate and complete OIDC login.
The former public-shell and proxy-authentication decisions no longer apply.

This keeps API validation, conditional writes, audit and convergence in the daemon.
The UI does not simulate runtime readiness, retain write-only credentials or
establish independent state in browser storage.
