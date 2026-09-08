# Implementation Status (Phase Boundary)

Status: Partial implementation; multi-account GitHub authentication (spec 0011 /
ADR-0015), the HTTP state backend adapter and the explicit Runtime adapter are
implemented with local verification; production lifecycle-worker integration,
real-GitHub multi-account acceptance and the production migration pending.
Evidence baseline: `b007fbc` (HTTP-backend increment), the Nix packaging and
NixOS deployment increment of 2026-09-07, and the multi-account authentication
increment of 2026-09-07; historical review reports dated 2026-09-06.

This is the sole implementation-progress record. [The documentation index](README.md)
separates accepted contracts, open decisions and release gates. A specification or
accepted ADR is not proof that its implementation or external acceptance is complete.
The earlier documentation-only pass could not run Cargo. The HTTP-backend pass
used an owner-approved temporary Nix Rust environment. The repository now provides
a locked flake development/build environment; verification and remaining
integration boundaries are recorded below.

## Multi-account GitHub authentication (spec 0011 / ADR-0015): local implementation (2026-09-07)

The authentication increment extends the existing control plane, supervisor and
GitHub adapter. It does not close the production lifecycle integration gaps
listed below. The accepted spec remains the release contract.

- **Policy and publication:** one numeric GitHub App identity and credential per
  profile, with typed organization, exact repository and user/organization
  account-repository selectors. Policies use case-insensitive canonical matching,
  reject ambiguous routes and remain intersected with installation access and
  runner permissions. Account selectors admit future owned repositories on demand;
  they neither create Fleets nor expand GitHub installation permissions.
- **Validation and promotion:** every declared account and exact target is checked
  through the GitHub API adapter. Numeric App/account/repository/owner identity,
  permissions and installation suspension are verified. Legacy client-ID upgrades
  prove same-App continuity through `/app`. Candidate rejection leaves the active
  revision untouched; transient and rate-limit failures retry with per-revision
  deadlines. Promotion atomically freezes bindings and the validation snapshot,
  rechecks all live dependencies and their mutation fences, and retargets handoffs.
- **Persistence and retention:** migrations m0008/m0009 add versioned policies,
  frozen bindings, Fleet contexts, immutable context history and session auth
  references. Legacy credential and replay encodings keep their original meaning;
  older binaries refuse the new durable format. Dependencies include desired and
  observed Fleet references, sessions, generations and unfinished operations.
  Cleanup resolves each generation's original credential and context; missing or
  corrupt authority keeps the resource occupied instead of selecting a fallback.
- **Handoff and execution:** the desired credential proves a new route while the
  current execution client stays bound to its observed revision. Success and
  failure updates compare the captured auth reference, mutation fence and complete
  desired context. A stale result is discarded. Acknowledgement ends the tick;
  wiring rebuilds from the persisted observed context before new effects. Exact
  repository pins apply from first admission, and transfer or same-name recreation
  cannot silently replace an existing identity.
- **Runtime access:** per-route singleflight proofs have at most 60 seconds of
  positive and 15 seconds of negative freshness. Refresh verifies installation,
  numeric target identity and runner access; time spent waiting or refreshing does
  not extend old evidence. Observed access failures invalidate proofs. Requests
  recheck authorization after credential/connection waits and retries. Repository
  administration tokens are narrowed by the proven numeric repository ID.
- **HTTP and UI:** versioned views attribute active and desired policy, validation
  state, reasons and bindings to their exact revisions. Pure legacy reads keep
  their original shape. Typed policy previews include actual live Fleet coverage;
  unavailable impact data blocks publication rather than claiming no impact.
  Explicit legacy upgrades require the numeric App ID. Fleet details show the
  full desired/observed route identity and handoff status. Bounded, process-local
  runtime observations provide per-binding health; expired evidence or restart
  returns Unknown. Candidate validation is shown separately from current access.

Local verification covers real HTTP admission, the scheduled worker, SQLite
promotion and handoff transactions, supervisor-to-GitHub-adapter composition,
scripted GitHub responses, token request bodies, delayed-response races, legacy
migration/replay and browser interactions. Required verification commands are
`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo nextest run --manifest-path Cargo.toml --workspace test` and the full
unfiltered `cargo nextest run --manifest-path Cargo.toml --workspace`; the trailing
`test` is a name filter. Web gates are `npm run fmt:check`, `npm run lint`,
`npm run build` and `npm test`.

Final local results (2026-09-07): full workspace 382 passed, 2 ignored;
the required filtered run 312 passed, 72 skipped; web 24 passed. Strict
all-target Clippy, rustfmt, web formatting/lint/build and the 400-line Rust
module limit passed. The two ignored Rust tests require a verified Terraform
1.9.8 binary and the separate long-running HTTPS/OIDC browser harness.

Open release gates: real GitHub multi-account acceptance under spec 0011 section 9,
Go-oracle protocol acceptance and migration of the existing production
`indexyz-org` profile. No production credentials or deployment were used for the
local checks. The pre-existing production session listener, message acquisition
and ingestion, recovery and decommission integration gaps recorded below remain
open. Scripted-server evidence does not establish those end-to-end workflows.

## Existing implementation and local test coverage

- Multi-crate pure-Rust workspace with enforced dependency architecture
  (`cargo tree` gate tests; spec 0007 §2/§6) — no Go/FFI, no Kubernetes or
  Docker client in the production closure.
- Domain core: lifecycle state machine enforced at the persistence
  boundary, capacity arithmetic (saturating, bounded), Terraform
  saved-plan fail-closed admission (create/delete exactness, deferred/
  import/deposed/move rejection), fixed `shaula`/`shaula_result`
  envelopes with keyed `bindings_digest` (HMAC, bootstrap-injected key).
- SQLite ledger: fleets/revisions/changes/auth-handoffs/idempotency/
  audit/outbox/template/auth profiles and revisions (credential plaintext
  per ADR-0007/0009), artifacts, generations, operations, sessions with
  monotonic epochs, demand snapshots, idempotent job observations,
  acquisition intents, scale set ownership. Atomic composite commits.
- Fleet HTTP control plane: conditional PUT/DELETE (428/412/410/409/422),
  idempotent replay and conflict, OIDC-derived actors (401/403),
  sanitized problem responses, loopback-only management bind fail-closed.
- Profile control plane: artifact publication (digest-verified, archive
  safety, atomic), template publish → static validation (scan-driven
  Ready) → conformance attestation (subject digest match, fail-closed) →
  activation, auth rotation with staged activation (prior active kept).
- Scale Set wire adapter (reqwest): GitHub App JWT + installation token
  and PAT flows, registration → Actions Service bootstrap with expiry
  refresh, scale set lookup/create, sessions with `X-ScaleSetMaxCapacity`
  long-poll, ACK, acquire, JIT (redacted), inventory and safe removal
  with `JobStillRunning` classification. Fixture-tested against a
  scripted mock of the pinned `actions/scaleset` protocol.
- Periodic scan loop: outbox consumption and candidate static validation.
- Bootstrap: data-directory ownership lock (single-writer via a
  kernel-held `fd-lock` advisory lock on a lock file — the OS releases it
  unconditionally on process death, so takeover never depends on PID
  probing; a second live process fails closed), migration-before-serve
  ordering, supervised scan loop with graceful shutdown, Day-0 JSON
  telemetry (`shaula-observability`: EnvFilter + JSON layer, bounded
  in-process metric counters; no operation spans or OTLP export yet).

## Durable data compatibility policy (R8-03)

Create provenance now records archive identity (`artifact_digest`) separately
from executable file-tree identity (`template_material_digest`). Publication
retains the original `<digest>.tar.gz` beside the extracted directory. Restore
both together; an older artifact store can be repaired by re-uploading the
original archive with the same digest. Missing or altered archives fail closed.
Legacy provenance without `template_material_digest` remains readable: Destroy
checks its former file-tree digest against material reconstructed from the
Generation's pinned archive, without rewriting historical records.

The durable identity encoding — the hash tuple encoding behind request
idempotency hashes and attestation record ids, and the canonical
attestation subject JSON — is VERSIONED. Migration `m0005` stamps every
database with the format its rows were written under; `Store::migrate`
verifies the stamp after migrating.

- A database stamped with the CURRENT version loads normally.
- A database that already contains rows stamped LEGACY (written by an
  unsupported pre-release build whose identity encodings differed) makes
  startup FAIL CLOSED with a "rebuild the data directory" error. There
  is deliberately NO in-place conversion: replaying the ambiguous legacy
  encodings cannot be proven safe, and pre-release development data
  directories carry no upgrade guarantee.
- A database stamped NEWER than this build also fails closed ("upgrade
  the binary") so an old binary can never misread a newer ledger.
- Fresh databases are stamped with the current version and are
  unaffected.

## Review fixes verified locally (2026-09-06)

- Create verifies archive identity and executable material separately. A
  subprocess fixture exercises publication, prepare, plan, apply and post-state
  capture through the production Windows engine fence.
- Retiring retries re-run the GitHub removal gate. Retiring/DestroyPending work
  runs even with zero capacity excess, before new Creates. Access failures and
  unknown remote runners block ownership admission.
- Fleet replacement occupancy checks include the exact Template Revision in
  the transaction. Auth replacement commits compare the accepted desired head;
  concurrent conditional replacements have one winner. Template/Auth replay
  lookup precedes current-head preconditions.
- Fleet tasks run independently with separate global Create/Destroy budgets.
  Worker panic is isolated, all children are owned through shutdown, and initial
  remote reconciliation does not delay HTTP startup.
- Profile DELETE uses the documented resource paths and commits retirement,
  Change, audit, outbox and replay facts atomically. New references are refused
  in the Fleet commit transaction; existing exact references and protected
  material remain available. Retained Profile heads currently keep retirement
  `Blocked(ResourceInUse)`; automatic head release, retirement completion and GC
  are implementation gaps, not the intended terminal behavior.
- Credential destinations use parsed HTTPS URLs restricted to the GitHub
  Actions service domain; loopback requires explicit test construction.
  Count/entry disagreement in runner inventory fails closed without indexing.
  Handwritten query escaping was replaced with the URL library's structured API.
- Bundled image aliases resolve through the manifest's digest map. Protected
  tfvars use private temporary files and atomic publication without replacement.
- Auth validation is scheduled by the binary. PAT principal and GitHub App /
  installation identity are checked before target access and activation. The
  organization/repository x PAT/App matrix is tested against local HTTP servers;
  identity mismatch is rejected and transient failures leave validation pending.
  Endpoint contracts follow the official [GitHub App API](https://docs.github.com/en/rest/apps/apps)
  and [authenticated user API](https://docs.github.com/en/rest/users/users).

## HTTP state backend increment (2026-09-07)

Implemented as an **opt-in adapter**, not a production lifecycle switch:

- `shaula-core/src/state_backend/` owns the narrow state port, redacted capability /
  document / LockInfo types, bounded raw-v4 validation and safe error classes.
  State capabilities are independently random, state-only secrets; SQLite stores
  SHA-256 verifiers, with constant-time comparison. No management OIDC or worker
  control credential is accepted as state authority.
- `shaula-store-migration/src/m0007_http_state.rs` adds `generation_http_state`
  without backfilling existing Generations. `shaula-store/src/http_state/` inserts
  a **new** Generation and its backend ownership atomically, checks the Fleet
  head, and refuses any existing Generation, including an unmigrated legacy row.
  This is the storage half of admission, not a replacement for daemon capacity,
  material/attestation admission or the Fleet effect gate. The migration entry
  point explicitly wraps SQLite DDL and migration history in one transaction;
  fault injection verifies rollback preserves pre-existing schema and permits
  retry. This atomic schema upgrade is not a legacy local-state importer.
- LOCK/POST/UNLOCK reserve SQLite's writer before ownership reads. Owner/epoch /
  lock/revision checks and the conditional update share that transaction, across
  independent connection pools. Same serial requires byte-identical replay and
  does not increment backend revision; changed lineage, stale serial, unlocked
  writes, wrong ID, revoked credentials and sealed writes fail closed. Lock
  metadata is not identity; there is no expiry takeover, force-unlock or DELETE.
- The backend-only revoke/seal primitives retain Generation Occupancy. They do
  **not** establish descendant death, GitHub-safe removal, Destroy completion or
  authority to delete a Workspace. `note_create_starting` is only a missing-state
  fence and requires initialized state; the future worker must establish initial
  state/lineage before planning/start handover, rather than assume Terraform init
  or plan persisted it. Atomic terminal receipt + seal + Occupancy release is not
  yet implemented.
- `shaula-http/src/state_backend/` provides an independently bound loopback-only
  `StateServer`, authenticates before reading bodies, rechecks ownership at each
  store operation, and implements the standard GET/POST/LOCK/UNLOCK wire protocol.
  Responses are private/no-store; no generic request/SQL log exposes state, lock
  metadata or credentials. Current hard bounds: 16 MiB state, 16 KiB LockInfo,
  16 in-flight handlers and 30-second request deadline. Raw-v4 serials are bounded
  to `0..=i64::MAX`. Final operational limits remain tracked under D3.
- `shaula-template/src/http_backend.rs` and `runtime_backend.rs` add explicit
  `TemplateRuntime::with_http_backend`. The sole extra source file is fixed
  `shaula.backend.tf`; its exact contents are verified separately from artifact
  identity. Backend passwords go only through `TF_HTTP_PASSWORD`, not HCL/argv.
  Runtime-owned env keys and proxy overrides are refused in provider env; cached
  backend config cannot override the empty HCL. Native HCL `backend`/`cloud`
  keywords are conservatively rejected even in comments/strings, and JSON keys
  are decoded before checking overrides. HTTP init retains readonly provider
  locks. Existing local/emergency state and non-pristine Workspaces are refused
  and preserved, not overwritten or implicitly migrated. HTTP Create/Destroy
  require an apply-intent sink; that seam is not yet a worker control client.
- SQLite WAL, foreign keys, busy timeout and `synchronous=FULL` now apply on every
  pooled connection, and SQL logging is disabled. Full regression exposed an Auth
  replacement read-to-writer upgrade race (500 instead of 412): `auth_repo.rs`
  now obtains the writer via revision INSERT before reading the head. A controlled
  writer-contention regression is in `shaula-store/src/tests/auth_concurrency.rs`;
  the original HTTP race passed 20 consecutive runs after the fix.

Coverage is in the core/HTTP backend module tests, `shaula-store/src/http_state/tests/`,
`shaula-template/src/http_backend_tests.rs` and `shaula/tests/http_state_backend.rs`.
The latter exercises real SQLite over loopback HTTP and has an explicit ignored
Terraform test enabled with `SHAULA_TEST_TERRAFORM`. Verified Terraform **1.9.8**
completed locked init, inspected saved Create/Destroy plans, apply, state pull and
empty-state verification with builtin `terraform_data`; raw state persisted in
SQLite and the password was absent from cached backend config. This is protocol
acceptance, **not** a Runner Profile/GitHub/Kubernetes/Docker conformance attestation.

## Nix packaging and NixOS deployment increment (2026-09-07)

- `flake.nix` delegates to `nix/flake-module.nix`: flake-parts composes packages,
  a fenix Rust development shell, treefmt-nix and checks for x86_64/aarch64 Linux.
  `flake.lock`, `Cargo.lock` and the fixed-output npm cache pin build inputs.
  `nix/packages/shaula.nix` builds the release executable and embedded Vite UI
  offline, checks TypeScript, runs debug all-target Clippy/nextest, and enables
  the actual Terraform HTTP-backend probe. Compiler paths are remapped to avoid
  retaining the build toolchain in the executable's runtime closure.
- `nix/packages/terraform.nix` packages the exact vendor Terraform 1.9.8 binary,
  verified against published ZIP hashes, without stripping or rewriting it.
  BUSL-1.1 is explicitly separate from Shaula's Apache-2.0 license. This pin is
  protocol-test evidence, not bundled Profile conformance or a completed R1 tuple.
- `nix/modules/shaula.nix` exports `services.shaula`: systemd credentials, private
  bootstrap generation, persistent bindings key, DynamicUser/private state, a
  loopback listener and SIGINT/control-group shutdown. It neither opens a public
  firewall port nor enables the unintegrated worker/state control plane.
- `nix/tests/` evaluates module defaults/rejected secret settings and boots two
  NixOS VMs using the real release package. HTTPS OIDC/PKCE and JWT/scope checks,
  embedded assets, conditional/idempotent Profile persistence, redaction, CSRF,
  logout/session invalidation, restart/reboot persistence and missing-credential /
  unavailable-issuer startup refusal are tested. The issuer and PKI are isolated
  test fixtures; HTTP session coverage is not JavaScript browser rendering.
- `.github/workflows/nix.yml` runs the flake checks on a KVM-enabled x86_64 Linux
  runner with read-only repository permissions, pinned actions and no production
  secrets. A remote workflow result is not implied by local verification.

Commands, service configuration and acceptance boundaries are in [the Nix guide](nix.md).

## Staged (next phases; not yet wired or externally validated)

- `shaula job`, exec Driver, protected launch handoff, worker control capability /
  client, GitHub gate, Create-start handover, command budgets and restart fencing
  are not yet implemented. `serve` does not yet bind `StateServer` or issue worker
  claims; production still uses the daemon-owned local-state lifecycle. No new
  backend behavior is silently enabled for an existing Generation. The adapter
  tests above do not establish complete compliance with
  [spec 0010](specs/0010-lifecycle-worker-and-http-state-backend.md) /
  [ADR-0014](ard/0014-run-lifecycle-workers-with-a-database-http-state-backend.md).
  Complete worker/provider crash and backend-outage recovery, atomic terminal
  completion, backup/restore and an explicit legacy state importer remain release
  gates. `m0007` creates a schema, not a legacy-state migration.
- Mandatory management OIDC is implemented with local test coverage; acceptance
  against the actual deployment's registered Provider and API client remains a
  release gate. See the dedicated OIDC evidence section below.
- The per-Fleet capacity/ownership/cleanup supervisor and Auth validator are
  started by the binary. The production session listener, persist-before-ACK
  message ingestion/acquisition, online/busy inventory classification, operation
  recovery and complete Fleet decommission/tombstone workflow are still missing.
  Existing persistence primitives alone do not implement these workflows.
- End-to-end real-GitHub validation and the Go-oracle differential suite
  (`references/scaleset`, pinned commit) have not been executed.
- Bundled-profile conformance remains an activation gate. The Docker provider
  now has a real Terraform-generated lock for `kreuzwerker/docker` 3.0.2 and
  a real, privately imported shim image pin; the Kubernetes lock/image pins
  remain staged.
  `templates/docker/image/` supplies a non-root shim image recipe and tests
  the read/unlink/env-only JIT handoff. `scripts/docker-conformance/` supplies
  an external local-state Terraform smoke harness with protected diagnostics
  and explicit GitHub removal evidence; it never asserts full conformance or
  produces an activation attestation. The [2026-09-08 real smoke](evidence/docker-smoke-2026-09-08/README.md)
  created one container as the Shaula OS identity, observed GitHub online,
  completed an actual job, confirmed safe GitHub removal, and verified Terraform
  Destroy/empty state/container absence. This does not implement the independent
  exec Driver or worker/HTTP-backend recovery. Full exact-tuple conformance is
  still required before activation. See the [harness instructions](../scripts/docker-conformance/README.md).
- OTLP export pipeline and OTel SDK instrumentation (bounded in-process
  counters and their call sites exist; no span/export pipeline yet);
  remaining endpoint surface (notably pagination and artifact metadata GET),
  and Profile reference clearance/retention/GC. Template/Auth revision and
  attestation reads already have handlers in
  `crates/shaula-http/src/router/profile_reads.rs`; their existence is not
  evidence that the entire specified read representation is complete.

## Contract alignment and remaining evidence gaps

| Contract | Source evidence / remaining gap |
| --- | --- |
| Keep an unchanged old Template pin during capacity/no-op PUT | Already handled by `ControlPlane::resolve_admission_materials` in `crates/shaula-daemon/src/service_fleet_ops.rs`; do not list this wholesale as unimplemented. Spec 0002 clarifies new-reference versus unchanged-reference admission. |
| Zero-Occupancy barrier for changed `template_inputs` | Still missing from the audited replacement boundary; `crates/shaula-store/src/registry_impl/commits_fleet.rs` currently protects reference replacement but does not establish the newly explicit inputs-change rule. Requires transactional race tests with Generation/worker admission. |
| Lower max below current Busy/Occupancy | Normative behavior is stop new admission and retire safely, not reject solely for current Occupancy or kill Busy. Existing arithmetic does not prove complete listener/worker scale-down acceptance. |
| Reachable cross-Auth handoff with an idle session | Spec 0002 removes an idle session as an admission blocker while retaining the zero-Occupancy/effect barrier. Full production listener/handoff integration and idle-session/race acceptance remain outstanding. |
| Profile retirement self-head release | `service_profile_retirement.rs` and `shaula-store/src/registry_impl/retirement.rs` retain current heads. Spec 0005 §7.1's final self-reference release/Retired transaction and history-versus-runtime-reference tests are missing. |
| Binding reads | Existing core/Profile reads expose coarse `bindings_present`; spec 0005 §5.2 retains that conservative projection, not a new per-secret fingerprint map. Full manifest annotation/schema/redaction acceptance still needs verification. |
| Attestation integrity/evidence | Existing `service_profile_attestation.rs`, `service_attestation.rs` and core `registry/attestation_subject.rs` provide authority/subject handling. No independent signing PKI is required by the clarified contract; complete external report linkage, canonical compatibility and real-platform suite evidence are not established by local record-acceptance tests. |
| `bindings_digest` compatibility | Current `BindingsDigest::from_keyed_material` in `crates/shaula-core/src/template.rs` emits `bd1_` HMAC-SHA256; its material does not include Profile incarnation. D4 remains deferred. No new encoding or historical-record rewrite was made. |

## Mandatory management OIDC: implementation and evidence

- Provider-backed browser session renewal is accepted in spec 0013 / ADR-0017
  (2026-09-08). The documentation precedes implementation; refresh-token storage,
  guard renewal and new expiry acceptance are not yet implemented or verified.

- `crates/shaula/src/oidc_args.rs` and `main.rs` load required CLI/env settings
  and initialize discovery/JWKS before listeners or resource workers.
- `crates/shaula-http/src/oidc/` owns verification, browser login, opaque sessions,
  Origin/CSRF, logout, default route protection and private/no-store responses.
  Legacy actor/backend-token headers do not establish a management identity.
- Rust coverage is in `crates/shaula/tests/oidc_startup.rs`,
  `oidc_authentication.rs` and the HTTP crate's OIDC tests. Browser coverage is
  in `web/e2e/oidc.spec.ts`, backed by `crates/shaula/tests/oidc_browser.rs`.
  Prior reports recorded local HTTPS Provider/browser acceptance. The current
  implementation pass reruns Rust tests; the separately ignored Playwright/browser
  acceptance is not a new browser validation claim.
- The actual registered deployment Provider/browser/API acceptance is still
  unverified. Internal state capability authentication is now independently
  tested; worker control authentication remains unimplemented. Neither is a
  management OIDC fallback.

## Embedded operator UI (2026-09-06)

- `web/` provides React + Vite 8/Oxc and CLI-downloaded shadcn/ui components for
  Fleet management, template publication/revisions, GitHub authentication
  profiles and accepted change status. See spec 0008 and ADR 0012.
- The HTTP crate builds and embeds the production frontend into both debug
  and release binaries. UI document routes support direct navigation;
  missing APIs/assets are not rewritten to HTML.
- The authentication inventory (2026-09-08) uses the `auth.read`-protected
  `GET /github-auth-profiles` collection, sharing the redacted detail serializer.
  `/auth` lists existing connections, supports search/refresh and preserves direct
  detail links. Summaries show active targets separately from candidate revisions.
  This increment returns the full local collection under spec 0012 / ADR-0016;
  it does not introduce pagination or enumerate GitHub repositories.
  Regression entrypoints are `shaula/tests/auth_read_views/list.rs` (real
  Router/SQLite enumeration, ordering, authorization, redaction and read faults)
  and `web/tests/auth-list.spec.ts` / `mutations.spec.ts` (entry-page discovery,
  selection, search, refresh, candidate separation and create invalidation).
- The UI uses OIDC-derived browser sessions under spec 0009 / ADR-0013.
  The session read returns actor name/scopes and a CSRF response header.
  Browser code has no backend token, identity assertion, durable credential
  storage or independent desired state.
- Node/npm dependencies are build prerequisites only; runtime serving needs
  no frontend directory. Setup and verification commands are in `web/README.md`.
- UI availability does not resolve any of the daemon/runtime gaps listed above.

## Implementation verification (2026-09-07)

The earlier documentation-only pass checked Markdown links/fragments and diff
whitespace but lacked Cargo (exit 127). That environment blocker has been resolved
with a temporary Nix shell: rustc/clippy/rustfmt 1.97.1, Cargo 1.97.0 and nextest
0.9.140, without repository environment files or global configuration changes.
Frontend prerequisites were installed with `npm ci --prefix web` (lockfile unchanged).
Terraform 1.9.8 was downloaded to a temporary directory and checked against the
vendor's HTTPS-published SHA-256 checksum list, not installed globally.

Final results after the Auth transaction and migration rollback fixes:

| Command | Result |
| --- | --- |
| `cargo fmt --all` and `cargo fmt --all --check` | Passed |
| `cargo clippy` | Passed |
| `cargo clippy --workspace --all-targets -- -D warnings` | Passed |
| `cargo nextest run --manifest-path "Cargo.toml" --workspace test` | 218 passed; 44 filtered/ignored |
| `cargo nextest run --manifest-path Cargo.toml --workspace --no-fail-fast` | 260 passed; 2 ignored |
| `SHAULA_TEST_TERRAFORM=<verified-1.9.8> cargo test --workspace --test http_state_backend -- --include-ignored` | 2 passed, including the real Terraform roundtrip |

The required command's trailing `test` is a name filter, so the additional
unfiltered run is necessary to cover all ordinary integration tests. Its initial
failure exposed the Auth transaction race described above; the regression and
20-repeat original reproducer now pass. The migration rollback test also failed
before the transaction wrapper and passed after it, including retry/idempotency.
The two default-ignored cases are the
explicit Terraform probe (run separately) and the Playwright/OIDC browser suite
(not rerun here). Windows-only engine tests, real Runner Platforms, actual GitHub
and registered OIDC Provider acceptance are not established by this Linux increment.

## Nix verification (2026-09-07)

Verified locally on x86_64 Linux with Nix 2.35.1, sandboxed builds, KVM and the
locked fenix stable Rust 1.98.1 toolchain:

| Command / check | Result |
| --- | --- |
| `nix fmt -- --clear-cache --fail-on-change` | Passed; no formatting changes, including actionlint and static lints |
| `nix flake check --print-build-logs` | Passed: release package/Rust tests, treefmt, module evaluation and both NixOS VMs |
| `nix flake check --all-systems --no-build` | Passed evaluation for x86_64/aarch64 Linux; not an aarch64 build/run |
| `cargo fmt --all`, `cargo fmt --all --check`, `cargo clippy` and all-target Clippy with `-D warnings`, inside `nix develop` | Passed |
| Required filtered / additional unfiltered workspace nextest, inside `nix develop` | 218 passed / 260 passed, respectively; same 44 / 2 skipped cases as above |
| Terraform HTTP-backend probe in the Nix package check | 2 passed, including actual Terraform 1.9.8 |
| Installed binary / runtime closure | `shaula 0.1.0`; about 59 MiB, without Node or the Rust toolchain |
| Packaged Terraform bytes | Identical to the independently checksum-verified vendor 1.9.8 binary |

The VM suite was rerun with the final release package after compiler-path
remapping. Formatting runs put auto-fix linters before whitespace formatters;
cache-cleared repeated runs and the sandboxed diff check pass. No Rust source or
web source was changed for this Nix increment. aarch64 native execution and a
pushed GitHub workflow run have **not** been observed. None of these checks closes
the real-Provider, GitHub, Runner Platform or worker-integration gates above.

## Known accepted limitations (per ADR)

- Same-Runner-Execution-Domain JIT process-inspection risk (ADR-0004).
- Name-based Kubernetes deletion residual risk (ADR-0006).
- Single ambient host-admin trust domain for IaC children (ADR-0008).
