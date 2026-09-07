# OIDC deployment

Shaula requires one OIDC Provider at startup. Register a confidential web client
supporting Authorization Code, PKCE S256, `openid`, RS256 and
`client_secret_basic`. Register the exact callback
`https://shaula.example.com/auth/oidc/callback`. Configure a separate API audience
and API clients that receive RFC 9068 JWT access tokens (`typ=at+jwt`, RS256).
ID tokens and opaque access tokens cannot authenticate API calls.
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
      scopes: [fleet.read, fleet.write, fleet.retire, template.read, template.publish, template.attest, template.retire, auth.read, auth.write, auth.retire]
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

## HTTPS ingress

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

Browser sessions last at most one hour or the ID token lifetime, with a
15-minute idle timeout. Logout and restart invalidate local sessions. The UI
keeps CSRF only in memory, clears query caches and credential forms after a 401,
and requires explicit sign-in without replaying a failed write. Logging out of
Shaula does not terminate the Provider's SSO session.

Metadata/JWKS are cached for five minutes. Unknown-key refresh is serialized and
backed off for ten seconds. Network requests have five-second connection and
ten-second total timeouts and a 1 MiB response limit. Stores hold at most 4,096
transactions and 4,096 sessions; code exchanges have a concurrency limit of 16.
Valid local sessions and still-valid known keys can survive a Provider outage;
expired keys never authenticate. Readiness reports observed authentication
failures while existing resource cleanup continues.

## Verification

`npm run test:oidc --prefix web` launches the real debug binary behind a local
HTTPS test proxy and performs authorization-code login with a TLS-verified
test Provider. It verifies embedded assets, resource pages, CSRF and logout.
The Provider fixture exists only in test binaries. Rust tests also cover startup
failures, grants, JWT rejection, key rotation/outage, cache and session bounds.

Before deployment acceptance, repeat browser login/logout and an API access
token request against the actual registered Provider and production HTTPS
origin. The local fixture does not establish that registration or token issuance
is correct in an external Provider.
