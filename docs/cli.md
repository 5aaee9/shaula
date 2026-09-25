# Remote CLI and personal access tokens

`shaula` includes an API client alongside `serve`. Remote commands never start a
daemon, open its database, or run Terraform. `shaula-client` and
`shaula-api-types` can be built independently of the server.

## Login

Use the OIDC browser session to open **Access tokens**. Select only the scopes
needed by the client, create a token, and save its one-time secret. Token
management requires explicit `access-token.read`, `access-token.write`, and
`access-token.revoke` grants; existing operators do not acquire these implicitly.
Only a primary OIDC session or API JWT can issue tokens. A PAT cannot delegate.

```console
shaula --server https://shaula.example.com --context work auth login --token-stdin
shaula auth whoami
shaula fleets list
shaula auth logout
```

Supply the secret through stdin or `--token-file`, never argv. The client also
accepts `SHAULA_ACCESS_TOKEN`. Explicit stdin/file wins over this environment
variable, then the selected context's private credential file. Server and
context flags override `SHAULA_SERVER` / `SHAULA_CONTEXT`, then configuration.
Use `--client-config` to select a TOML configuration file; the default is
`%APPDATA%/shaula/client.toml` or `$XDG_CONFIG_HOME/shaula/client.toml` (otherwise
`$HOME/.config/shaula/client.toml`). Contexts contain only the origin, credential
kind and file reference. Login creates a private credential file; Unix requires
0600 and Windows grants access only to its owner. Symlink/reparse paths fail.
Logout removes the local credential; `auth logout --revoke` first revokes the PAT.

For an externally obtained primary API JWT use `--credential-kind oidc`. No
browser cookie or Provider refresh token is imported. HTTPS is mandatory;
`--allow-loopback-http` explicitly permits an IP loopback tunnel. Redirects are
never followed. A client talking to an old server can use OIDC for business
operations; without its stable principal projection, retries bind to the exact
credential and cannot migrate across credentials.

## Resource operations

```console
shaula templates sources list
shaula templates variables <artifact-digest>
shaula templates input-contract <profile> <revision>
shaula fleets create <key> --file fleet.json
shaula fleets get <key>
shaula fleets update <key> --file fleet.json --if-match '<exact quoted version>' --wait
shaula fleets edit <key>
shaula fleets retire <key> --if-match '<exact quoted version>' --yes --wait
shaula templates revisions get <profile> <revision>
shaula templates revisions publish <profile> --file revision.json --if-match '<exact quoted version>'
shaula templates update <profile> --file update.json --if-match '<exact quoted version>'
shaula pools create <key> --file pool.json
shaula auth-profiles create <key> --file auth.json
shaula auth-profiles rotate <key> --file auth.json --if-match '<exact quoted version>' --yes
shaula auth-profiles policy-update <key> --file policy.json --if-match '<exact quoted version>' --yes
shaula auth-profiles installation-link <key>
```

JSON files use the HTTP request schemas in the specs. Fleet writes are complete
documents, preserving unedited properties and numeric tokens. `edit` retains the
original version and uses `$VISUAL` / `$EDITOR` with a restricted environment;
concurrent updates yield 412. Template Update omits bindings to inherit them;
top-level null is rejected, and per-field keep sentinels retain their meaning.
Auth rotation/policy changes display their impact before confirmation. Secrets
in Auth request files remain write-only. Installation links are displayed only.

Reads put the endpoint's JSON in `data`, and opaque resource versions in
`metadata.version`. Copy the entire quoted version into `--if-match`; never
construct it from a revision. Mutations use one idempotency key and submit once.
Reuse `--idempotency-key` with exactly the same body and precondition to recover
an uncertain result. A 202 receipt means accepted; `--wait` succeeds only on
Converged. Tracking errors retain the receipt. Finalize has a separate,
ledger-only receipt and does not follow a nonexistent Change endpoint.

## History and logs

```console
shaula jobs list --fleet <key> --all --max-items 1000
shaula jobs get <id>
shaula generations list --association unassigned
shaula invocations list <generation-id> --all
shaula logs read <invocation-id> --phase apply --stream stderr --follow --timeout 5m
shaula logs download <invocation-id> --file logs.jsonl
shaula changes wait <id> --kind fleet --timeout 5m
shaula generations finalize <id> --reason '<out-of-band evidence>' --confirmed-absent --yes
```

Time filters use UTC Unix milliseconds. Pagination has item/time budgets and
retains partial results. Invocation `operation` distinguishes Create/Destroy;
log `phase` remains init/plan/apply. Downloads are JSON Lines retaining sanitized
entries, capture status, content version, gaps and lost bytes. A content-version
change stops follow rather than joining incompatible histories. Finalize requires
out-of-band absence evidence and does not delete infrastructure.

## Issuance and rotation

```console
shaula --credential-kind oidc --token-file primary.jwt tokens create --name workstation --scope fleet.read --ttl 30d --secret-out new.token
shaula tokens current
shaula --credential-kind oidc --token-file primary.jwt tokens rotate <old-id> --name workstation --scope fleet.read --ttl 30d --secret-out replacement.token
```

Secret output files must not already exist and their parent directory must grant
access only to the current user (0700 on Unix; owner-only ACL on Windows).
`auth login` creates its default private credential directory automatically.
`--show-secret` is the explicit
alternative for a one-time issuance response. Replaying a committed issue returns
metadata without the secret (exit 10): revoke the unrecoverable token and issue
with a new key. Rotation saves and verifies the replacement first. Add
`--revoke-old --if-match '<old version>' --yes` to revoke the old token afterward;
failure leaves the new credential file available and reports the unresolved step.

Noninteractive output defaults to JSON. `--output json` uses six fields:
`schema_version`, `command`, `data`, `metadata`, `receipt`, `error`. Watch/follow
uses JSON Lines with a final `end` event. Exit codes: 0 success/accepted; 2 local
input; 3 authentication; 4 scope; 5 conflict/precondition; 6 absent; 7 validation;
8 tracking; 9 unavailable/uncertain; 10 secret delivery/recovery; 130 cancellation.
See [implementation status](IMPLEMENTATION_STATUS.md) for verified coverage and
remaining deployment acceptance; route coverage alone is not workflow parity.
