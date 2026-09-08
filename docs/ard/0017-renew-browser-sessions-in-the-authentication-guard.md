---
status: accepted
date: 2026-09-08
amends: [0013]
---

# Renew browser sessions in the authentication guard

## Context

The original management OIDC contract discarded refresh tokens and deleted local
sessions at ID Token / one-hour expiry or fifteen-minute idle. The next API 401
made React clear its cache and unmount the current page and credential editor.
The user requires automatic Provider renewal that leaves the page unchanged,
with total renewal lifetime controlled entirely by the Provider.

## Decision

The server-side OIDC module retains refresh tokens in its bounded memory store.
The existing request guard renews a stale authentication lease before admitting
the original request to a business handler. Detailed lifetimes, token validation,
concurrency, failures and acceptance belong solely to
[spec 0013](../specs/0013-provider-backed-browser-session-renewal.md).

Background renewal preserves the opaque session ID and CSRF value. Complete new
login still rotates both. Refreshable browser cookies outlive the short lease as
session cookies; there is no additional Shaula total-duration limit. A Provider
without refresh tokens retains the original finite-session behavior.

Each session shares an owned, bounded renewal operation. Network I/O never holds
the global store lock, and commit checks session membership so cancellation cannot
lose token rotation and logout cannot be undone by a late completion. Unsafe
request Origin/CSRF validation precedes renewal. Logout only revokes locally.

OIDC refresh responses may omit ID Tokens. The authenticated token exchange then
renews the original identity for its declared access-token lifetime; optional new
ID Tokens are fully verified and bound to the original authentication. Static
Shaula grants remain authoritative. Tokens stay out of browser state and storage.

## Alternatives considered

- A frontend refresh endpoint and shared 401 retry promise add another public
  auth route, response generations, request replay and form-preservation logic.
  They also do not by themselves handle expired document/asset requests. The
  existing guard can cover these paths without replaying any request.
- Redirecting through Provider SSO, or reloading after renewal, destroys mounted
  editor state and does not meet the requested successful-renewal behavior.
- Requiring a new ID Token on every refresh rejects conforming Providers that
  omit it. Calling UserInfo adds another failure boundary without being required
  to retain an identity already bound to this refresh grant.
- A fixed eight-hour/day limit or hard idle logout contradicts the selected
  Provider-controlled lifetime. Persisting tokens across daemon/browser restarts
  introduces storage and key-management scope not needed for this feature.

## Consequences

The first request after lease expiry may wait for Provider I/O. Temporary failures
return 503 without granting stale access or marking the frontend logged out;
terminal failure retains the existing login fallback. Refresh-token rotation after
a lost response cannot be recovered locally and may require a new login.

Refreshable sessions with opaque, unknown Provider expiry may occupy bounded slots
until logout, a terminal exchange or restart. At capacity new login fails rather
than growing memory indefinitely. Deploying this change clears existing in-memory
sessions, so one complete login is needed to obtain refresh material.

This amends ADR-0013's expiry behavior only. Mandatory OIDC, anonymous-route limits,
CSRF, no-store, authorization, machine bearer validation and internal worker
credential separation remain governed by their existing contracts. Implementation
and actual Provider validation are recorded in [implementation status](../IMPLEMENTATION_STATUS.md).
