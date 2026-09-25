# OIDC deployment

This guide covers the management listener (UI/API/health). The target internal
worker/state listener in [spec 0010](specs/0010-lifecycle-worker-and-http-state-backend.md)
uses separate Generation-scoped capabilities and must not be exposed by this
reverse proxy. The state adapter exists but is not wired into `serve`; worker
control remains unimplemented. See [implementation status](IMPLEMENTATION_STATUS.md).
For the credential-aware NixOS service module and VM acceptance, see
[Nix deployment](nix.md).

Shaula requires one OIDC Provider at startup. Register a confidential web client
supporting Authorization Code, PKCE S256, `openid`, RS256 and
`client_secret_basic`. Register the exact callback
`https://shaula.example.com/auth/oidc/callback`. Configure a separate API audience
and API clients that receive RFC 9068 JWT access tokens (`typ=at+jwt`, RS256).
ID tokens and opaque Provider access tokens cannot authenticate API calls.
Shaula-issued personal access tokens are a separate API credential described below.
The browser login also requests `profile` when Discovery advertises that scope.

## Startup

Provide the following through your service manager's environment. The client
secret has no command-line flag and must never use a `VITE_` variable.

```dotenv
SHAULA_OIDC_PROVIDER=https://identity.example.com/realms/operations
SHAULA_OIDC_CLIENT_ID=shaula-web
SHAULA_OIDC_CLIENT_SECRET=<configured by the secret manager>
SHAULA_OIDC_PUBLIC_URL=https://shaula.example.com
SHAULA_OIDC_API_AUDIENCE=shaula-api
```

The four non-secret required values also accept `--oidc-provider`,
`--oidc-client-id`, `--oidc-public-url`, and `--oidc-api-audience`. Explicit flags
override environment values, including an explicitly empty value, which fails.
Private Providers can add a trusted PEM CA using `--oidc-ca-cert` or
`SHAULA_OIDC_CA_CERT`; HTTPS and hostname verification remain mandatory.

Add explicit grants to the existing bootstrap file:

```yaml
http:
  listen: 127.0.0.1:8080
  bindings_server_key: <existing unique deployment key>
  authorization:
    - issuer: https://identity.example.com/realms/operations
      subject: <exact user sub claim>
      scopes: [fleet.read, logs.read, fleet.write, fleet.retire, template.read, template.publish, template.attest, template.retire, auth.read, auth.write, auth.retire]
    - issuer: https://identity.example.com/realms/operations
      subject: <exact automation sub claim>
      scopes: [fleet.read, template.read]
```

Retain the existing `storage` and `execution` configuration. Remove the obsolete
`http.backend_token`; unknown bootstrap fields fail validation. Neither display
names nor emails identify grants. Unmapped identities can read their own session
and health, but receive `403` for resource operations. API scopes are the
intersection of the server grant and the verified access token's `scope` claim.

Run `shaula serve --config bootstrap.yaml`. Discovery and JWKS must initialize
successfully before the daemon creates runtime state, starts workers or listens.
Provider/client settings are not accepted in bootstrap YAML.

## Personal access tokens

Add `access-token.read`, `access-token.write`, and `access-token.revoke` explicitly
to the intended operator's `http.authorization[].scopes`. They are not implicit
administrator privileges. The write scope permits issuance only from primary
OIDC authentication and cannot be delegated to a PAT. For primary API JWT
issuance the Provider's scope claim must also contain `access-token.write`.

Optional bootstrap policy (defaults shown):

```yaml
http:
  access_tokens:
    enabled: true
    default_ttl_secs: 7776000
    max_ttl_secs: 31536000
    max_active_per_principal: 20
    max_active_total: 10000
```

Merge this into the existing `http` object. TTL is at least 60 seconds and at
most 365 days. Issuance is also bounded to 5 new tokens per owner/minute and
100 globally/minute. Metadata-only idempotent recovery does not consume quota.
Disabling PATs rejects issuance/authentication while primary OIDC can still read
metadata and revoke. Current grants are intersected on every PAT request, with
no positive authentication cache. Grant configuration changes require restart.
IdP logout/disable does not remotely revoke local PATs; explicitly revoke, remove
the grant and restart, or disable PATs. Runtime Provider outage does not prevent
local PAT verification, while OIDC readiness remains independent. Discovery/JWKS
failure at startup still prevents the daemon and workers from starting.

Before upgrade, stop service and back up the entire data directory (including
SQLite WAL if not checkpointed), then verify the backup. Migrations 23–25 add
principal/operation idempotency, token storage and audit provenance. Legacy
idempotency records with unknown owner return `LegacyIdempotencyConflict`; they
are never attributed to a retrying caller. Roll back binary and the matching
complete pre-upgrade data together; do not run an old binary against the new
idempotency schema.

After restoring any backup, keep ingress closed and start with
`http.access_tokens.enabled: false`. A restored database may resurrect revoked
tokens. Have each owner revoke their imported tokens through primary OIDC, or
perform a reviewed offline administrative transaction revoking every imported
token, incrementing revisions and appending recovery audit. Back up first and
verify the complete revoked set before re-enabling PATs and issuing replacements.
There is no ordinary cross-owner token administration endpoint. If complete
revocation cannot be proven, leave PATs disabled.

See [CLI usage](cli.md) and [acceptance status](IMPLEMENTATION_STATUS.md).

## HTTPS ingress configuration

Terminate HTTPS at a reverse proxy forwarding to the loopback listener. Forward
cookies, Authorization, Origin and X-CSRF-Token without generating identity
headers. Preserve the browser's configured origin. Disable response caching and
access-log query/cookie/header capture on OIDC routes; authorization codes occur
in the callback query. A minimal Caddy route is:

```caddyfile
shaula.example.com {
    reverse_proxy 127.0.0.1:8080
}
```

Only exact GET login/callback routes permit anonymous access. UI documents
redirect to login; assets, API, probes, unsupported methods and unknown paths
require credentials. All responses are `Cache-Control: private, no-store`.
Do not configure an anonymous probe exception. Supply a valid API access token
to `/livez` and `/readyz` through the probe's secret configuration.

When the Provider supplies a refresh token, Shaula keeps it only in daemon memory
and renews expired browser leases in the authentication guard before executing
the original request. The short lease is capped at one hour and the supplied
ID/access-token lifetime; fifteen-minute idle also triggers renewal. The Provider
controls total renewal lifetime, with no extra Shaula day/hour limit. Refreshable
cookies are browser session cookies, so browser closure can still discard them.
Successful renewal preserves the page, open drafts, session ID and CSRF value.
No frontend refresh/replay or additional `offline_access` request is used.

Without a refresh token, the original one-hour / ID-token expiry and fifteen-minute
idle limits still require login again. Logout and daemon restart invalidate both
kinds of local session. A deployment restart requires one full login to obtain
new refresh material. The UI keeps CSRF only in memory and clears query caches and
credential forms after a terminal 401 or logout. Temporary renewal failures return
503 with a ten-second server backoff; they never grant stale access. The existing
error UI may replace an editor after a failed request. Logging out of Shaula does
not terminate the Provider's SSO session. See [spec 0013](specs/0013-provider-backed-browser-session-renewal.md)
for identity validation, rotation, lost responses and Provider compatibility.

Metadata/JWKS are cached for five minutes. Unknown-key refresh is serialized and
backed off for ten seconds. Network requests have five-second connection and
ten-second total timeouts and a 1 MiB response limit. Stores hold at most 4,096
transactions and 4,096 sessions; login and refresh exchanges share a concurrency
limit of 16. Unknown refresh-token expiry is not guessed: retained sessions can
occupy slots until revoked, replaced, rejected by the Provider or cleared by restart.
At capacity a new login returns 503 rather than evicting an existing user.
Valid local sessions and still-valid known keys can survive a Provider outage;
expired keys never authenticate. Readiness reports observed authentication
failures while existing resource cleanup continues.

## Verification

`npm run test:oidc --prefix web` launches the real debug binary behind a local
HTTPS test proxy and performs authorization-code login with a TLS-verified
test Provider. It verifies embedded assets, resource pages, CSRF, logout and
short-lease renewal while preserving the same page and mounted draft input.
The Provider fixture exists only in test binaries. Rust tests also cover startup
failures, grants, JWT rejection, key rotation/outage, cache and session bounds.

Before deployment acceptance, repeat browser login/logout and an API access
token request against the actual registered Provider and production HTTPS
origin. The local fixture does not establish that registration or token issuance
is correct in an external Provider.
