# Embedded Web UI

The operator UI lives in `web/` and is served from `/` by the existing loopback
HTTP listener. It uses React, Vite 8's Oxc toolchain and official shadcn/ui
components downloaded with the CLI and composed into resource views.
The UI is an API client, not a second source of Fleet/Profile desired state.

## Contracts

The multi-account GitHub authentication form and per-binding status
are specified in [spec 0011 §6](0011-multi-account-github-authentication.md#6-http-and-ui-contract).
Authentication inventory and detail discovery are specified in
[spec 0012](0012-github-authentication-inventory.md). Implementation and verification
status belongs in [IMPLEMENTATION_STATUS.md](../IMPLEMENTATION_STATUS.md).

- Cargo builds and embeds the frontend into debug and release binaries. A failed
  frontend build fails the binary build. Runtime serving requires no frontend files.
- UI document routes support refresh and direct links. Unknown API paths and
  missing static assets remain errors, never successful SPA documents.
- The UI shell, deep links and all embedded assets require an OIDC-derived
  browser session under [spec 0009](0009-mandatory-openid-connect.md) and
  [ADR-0013](../ard/0013-require-openid-connect-for-all-http-access.md). `serve`
  requires Provider/client configuration through clap/env and fails before
  listening when it is missing or invalid. The public shell is removed.
- Unauthenticated UI document GETs redirect to OIDC login before sending HTML.
  API, asset and HEAD requests return 401; unauthorized authenticated API
  requests return 403. Authentication precedes routing fallback and cache checks.
  All responses use private/no-store, including hashed assets; no service worker
  or offline cache may retain the protected UI.
- Rust owns login, callback, token validation and the opaque HttpOnly/Secure
  cookie session. Same-origin browser requests carry no backend token or
  caller-asserted actor; proxy identity injection and development bypasses are
  removed. React never receives OIDC tokens or the client secret.
- `GET /api/v1/session` returns `{name, scopes}` and `X-CSRF-Token` in a response
  header. UI mutations echo the CSRF header; the server validates it together
  with Origin. A session menu supports authenticated, CSRF-protected logout.
  Lease expiry first renews through the Provider in the server guard under
  [spec 0013](0013-provider-backed-browser-session-renewal.md). Successful renewal
  preserves the mounted page, URL and draft state. Terminal 401 or logout clears
  in-memory resource/query and credential form state; authentication failure must
  not replay a mutation. Temporary renewal 503 does not mark authentication expired.
- UI creates, edits and retirement preserve conditional mutation and idempotency
  semantics. An editor keeps the version originally displayed even as background
  queries refresh. Conflicts retain user input and require explicit reload/review.
- Creation dialogs start with the required identity, source/target and credential
  fields. Optional settings live in a collapsed **Advanced settings** section;
  collapsing preserves values and submission semantics. Hidden invalid controls
  expand their section before native validation focuses them; invalid advanced
  JSON also reveals its editor. Existing resource values are never reset by folding.
  New Fleets default the scale set name to their key unless explicitly overridden;
  runner group `Default`, zero minimum runners, empty labels/inputs and the active
  template revision remain defaults. Maximum capacity stays visible. Templates
  expose an archive/existing-digest choice; Terraform, bindings and input policy
  are advanced. Authentication retains a visible first target and required
  credentials; additional targets may fold, but policy-change previews and live
  Fleet impact/errors stay visible when modifying an existing policy. Folding
  does not infer broader authorization or supply unknown template-specific bindings.
- Accepted changes are polled and display their actual state. UI readiness or
  `202` acceptance does not establish runner provisioning or daemon completeness.
- Credentials/bindings are transient write-only form inputs. They are not stored
  in browser storage or included in query URLs, analytics or error reporting.
- Loading, empty, permission-denied, disconnected and conflict states are visible.
  Read/write/retire controls respect the session's scopes; backend authorization
  remains authoritative.

See [web development and deployment](../../web/README.md) for commands and scope.
The README describes development/deployment commands, not a second authentication
contract. Current implementation coverage and outstanding real-Provider acceptance
are recorded only in [implementation status](../IMPLEMENTATION_STATUS.md).
