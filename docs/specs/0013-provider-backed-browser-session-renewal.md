# Provider-backed browser session renewal

- Status: Accepted contract; implementation evidence belongs in [implementation status](../IMPLEMENTATION_STATUS.md)
- Date: 2026-09-08
- Decision: [ADR-0017](../ard/0017-renew-browser-sessions-in-the-authentication-guard.md)
- Amends: [spec 0009 §4/6](0009-mandatory-openid-connect.md), [spec 0008](0008-embedded-web-ui.md)

## 1. User outcome and scope

A browser session whose short authentication lease or idle threshold has expired
MUST first attempt renewal using its Provider-issued refresh token. Successful
renewal continues the original HTTP request before its handler runs. The open
page, pathname/query/hash, mounted dialogs, form drafts and query cache MUST remain
in place. No browser navigation, application reload or mutation replay is needed.

This concerns management OIDC login only. GitHub authentication connections,
installation tokens, runner sessions and machine bearer-token renewal are separate.
The exact anonymous login/callback allowlist and all existing authorization,
Origin/CSRF, idempotency and conditional-write checks remain mandatory.

## 2. Lifetime and retained credentials

The Provider controls the total renewal lifetime. Shaula MUST NOT impose an
additional eight-hour, day-long or other fixed/sliding total authentication limit.
Logout, daemon restart and replacement by a complete new login still revoke the
local session. Browser closure may discard its session cookie; this feature does
not promise persistence across browser restarts.

A refreshable session retains the opaque session ID, stable CSRF value, verified
identity and original OIDC binding, OAuth scopes, refresh token and short lease in
daemon memory only. Tokens MUST NOT enter SQLite, cookies, frontend responses,
browser storage, logs or Debug output. Access tokens need not be retained.

The short lease ends at the earliest of one hour, the current ID Token expiry
(when supplied) and a supplied access-token `expires_in`. Fifteen minutes without
an authenticated request also requires Provider renewal; it is not a hard logout.
Provider renewal is required before any protected content or effect is admitted
after either threshold. Only successfully authenticated requests extend idle time.

Refreshable sessions use a Secure/HttpOnly/SameSite=Lax/Path=/, no-Domain
`__Host-shaula-session` session cookie without Max-Age/Expires; the browser must
not discard it at the short lease boundary. The cookie never proves freshness by
itself. Successful background renewal keeps session ID and CSRF unchanged.

If login supplies no refresh token, preserve spec 0009's original hard lease
(at most one hour / ID Token expiry, fifteen-minute idle) and expiring cookie.
Provider support for refresh MUST NOT be required for ordinary login. Continue
requesting `openid` and advertised `profile`; do not silently add `offline_access`
or force additional consent. Providers requiring a separate consent configuration
remain subject to the no-refresh fallback until explicitly configured elsewhere.

Login transactions and sessions remain separately capped at 4,096. Expired
non-refreshable sessions are evicted; retained refreshable sessions are removed on
logout, replacement, terminal renewal failure or shutdown. Their unknown Provider
expiry cannot be inferred from opaque tokens. At capacity reject new sessions with
503 rather than silently evict existing users or retain unbounded entries.

## 3. Provider exchange and identity binding

Use OAuth 2.0 `grant_type=refresh_token` with the same confidential client and
validated token endpoint as login. Retain TLS verification, no redirects, bounded
response bodies, timeouts and the shared sixteen-exchange concurrency limit.
Renewal MUST NOT request additional scopes. If response scopes are supplied,
require `openid` and reject expansion beyond the original grant.

A successful response must carry a nonempty access token and a positive bounded
lease: either positive `expires_in` or a valid new ID Token's future expiry.
Replace the refresh token atomically when a nonempty new one is supplied; when
the field is omitted, retain the previous token as allowed by RFC 6749 §6. A
supplied empty refresh token is malformed and terminates renewal. Never expose
either token. Login likewise rejects a supplied empty refresh token.

OIDC Core §12.2 permits refresh responses without an ID Token. In that case a
successful authenticated refresh exchange with positive `expires_in` renews the
original Provider-bound identity; it cannot change subject or grants. No UserInfo
endpoint or decoding of an opaque access token is required.

If a new ID Token is supplied, validate its signature, configured exact issuer,
client audience/azp and time claims under spec 0009. Additionally require the same
subject and audience values as the original login, the original nonce if a nonce
is present, and unchanged original `auth_time` if it is present. An omitted nonce
is valid for refresh. Recompute authorization from the configured bootstrap grants;
Provider claims never independently grant Shaula permissions. An invalid returned
ID Token or malformed successful response terminates the session, including when
the Provider has already rotated its refresh token.
If validation cannot complete after a successful token response (for example,
JWKS retrieval fails), terminate the session with 401 rather than falling back
to the possibly consumed old token. The transient-error policy below applies to
failures before a successful token response has been received.

## 4. Request admission, concurrency and failures

The existing authentication guard owns renewal. It validates cookie syntax and
credential exclusivity, and for unsafe requests exact Origin and session-bound
CSRF, before attempting a Provider exchange. It then ensures a fresh identity
before forwarding the original request body to the handler exactly once.
No public refresh endpoint or frontend 401/retry state machine is introduced.

Concurrent requests sharing one stale session MUST share one renewal result.
No global session-store lock may span Provider I/O; unrelated valid sessions and
logout must remain usable. A cancelled waiting request cannot discard an already
rotated token: a bounded owned renewal task completes independently and commits
only if that same local session is still registered. Logout and replacement fence
late completion; renewal cannot recreate a removed session.

Exact `POST /auth/oidc/logout` may revoke a retained stale session after valid
Origin/CSRF, without contacting the Provider. This narrow local-revocation path
does not authorize other stale-session requests. Logout never starts renewal.

| Result | Behavior before handler |
| --- | --- |
| Fresh local lease | Continue without token exchange |
| Successful renewal | Continue original request; stable cookie/CSRF; no redirect |
| Missing session/token, `invalid_grant`, revoked token, identity mismatch or malformed successful token response | Remove terminal session; API/assets/health 401, document GET retains the existing login fallback |
| Network failure, 429, Provider 5xx or exhausted exchange capacity | 503, no stale access; keep retained session with a ten-second retry backoff |

All waiters observe a failed exchange consistently; backoff prevents polling from
hammering the Provider. A lost network response may have consumed a rotating token:
Shaula cannot reconstruct it. A later `invalid_grant` ends the session; availability
is not obtained by accepting stale identity. Invalid bearer or mixed cookie/bearer
requests never attempt browser renewal. Ordinary 403/409/412/business 5xx are not
authentication-refresh signals.

The frontend only treats terminal 401 as expired and clears sensitive state using
its existing explicit-sign-in flow. A renewal 503 is an ordinary recoverable error,
not a logout. Successful renewal produces a normal response, so URL, editor state,
ETag and idempotency key naturally remain intact. Interactive-login fallback does
not promise draft or fragment restoration; no sensitive draft persistence is added.
The existing error UI may replace resource editors during a temporary failure;
draft preservation is guaranteed for successful renewal, not failed data requests.

## 5. Acceptance

1. A short-lived Provider lease expires on an open real HTTPS browser page. The
   next session/data request performs refresh and succeeds without navigation;
   pathname/query/hash and the same mounted draft input retain their values.
2. A stale-session mutation with valid Origin/CSRF refreshes before admission and
   executes its handler exactly once. Invalid Origin/CSRF performs no exchange or
   effect. No client-side replay is added.
3. Concurrent stale reads share one exchange; cancellation of a waiter does not
   lose rotation. Unrelated sessions remain available. Logout or a complete new
   login during refresh prevents resurrection by late completion.
4. Cover refresh-token rotation and omission, ID Token omission, nonce omission,
   subject/issuer/audience/auth_time/nonce mismatch, scope expansion, invalid or
   missing expiry, invalid_grant, outage/backoff and no-refresh fallback.
5. Idle expiry renews; no Shaula total-time cutoff exists for refreshable sessions.
   Session cookies survive short expiry; stores remain bounded. Restart/logout
   invalidate sessions. Bearer validation and no-store/redaction remain intact.
6. Run rustfmt, clippy, full workspace nextest, frontend checks and the real daemon
   HTTPS browser suite. Test-only Provider controls MUST NOT become production
   configuration or authentication bypasses. Record mock-backed acceptance and
   actual registered-Provider acceptance separately.

## References

- [OIDC Core 1.0 §12](https://openid.net/specs/openid-connect-core-1_0.html#RefreshTokens)
- [OAuth 2.0 RFC 6749 §6](https://www.rfc-editor.org/rfc/rfc6749.html#section-6)
- [OAuth security BCP RFC 9700 §4.14](https://www.rfc-editor.org/rfc/rfc9700.html#section-4.14)
