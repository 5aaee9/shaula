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

## Jobs and retained operation logs: local implementation (2026-09-09)

[Spec 0019](specs/0019-workflow-jobs-and-operation-logs.md) and
[ARD-0023](ard/0023-retain-operation-logs-and-present-workflow-jobs.md) define
workflow-job-oriented Jobs, retained Apply/Destroy attempt logs, and optional
bootstrap delivery of the Create apply projection through `.setup_info`.

The implementation now includes:

- `shaula-core/src/jobs` and `shaula-store/src/jobs`: durable observations before
  ACK, opaque job identity, assignment reduction, exact numeric Runner association,
  frozen Generation scope, filtered pagination and bounded history expiry.
- `shaula-template/src/operation_capture.rs`, `operation_sanitize.rs` and
  `runtime_logging.rs`: bounded capture before output/error classification,
  record sanitization before the lossy queue, and separate runtime/capture results.
  `shaula-store/src/operation_logs` archives approved text separately from Workspace,
  checks content digests, preserves retries and implements quotas, recovery and GC.
- `shaula-http/src/router/jobs.rs` plus the embedded Jobs pages: read-only job,
  Generation and invocation history, independent `logs.read`, cursor-based log and
  invocation pages, explicit ambiguous/unavailable states and session-only browser
  retention. Embedded document routes and OIDC return targets include Jobs.
- `shaula-core/src/setup_info.rs`, `shaula-store/src/setup_info.rs` and the dedicated
  `shaula-http/src/setup_info` listener: expiring verifier-only capabilities issued
  before immutable v2 inputs, with current clock time, bounded requests and no
  management/state/control routes. `shaula/src/diagnostics.rs` wires these into the
  actual daemon and drains capture before releasing ownership.
- `templates/docker/image/setup_info.py` and `templates/setup-info/prepare.py`:
  bounded main-container bootstrap delivery and separate Docker/Kubernetes v2
  sources without changing existing image pins or resource cardinality.

Local validation on 2026-09-09: the full workspace nextest gate passed 579 tests
with 2 existing external-environment tests skipped; workspace Clippy and rustfmt
passed. The browser suite passed 82 tests, and Linux bootstrap/helper and v2 source
generation passed 21 and 2 tests respectively. Coverage includes Jobs
identity/reassignment and migration replay, archive retention/recovery/capture,
HTTP permissions, embedded routes and browser behavior. These checks are not real
Runner acceptance.

The current production composition still uses the daemon-owned local-state
Runtime; future Lifecycle Worker/HTTP-state integration remains staged. Existing
v1 image digests have not been rebuilt or relabeled. Enabling Setup Info requires
a newly built/distributed shim image with its actual repository digest, an
admitted v2 Template and the separate HTTPS delivery origin described in
[the operator guide](jobs-and-operation-logs.md). Docker and Kubernetes first-job
`Set up job` smoke acceptance for this increment has not been run. Older
generations have no retroactively generated logs or metadata.

## Fleet listener and Pending diagnosis repair (2026-09-08)

This increment closes the missing listener/message-ledger composition seam under
spec 0001 sections 7–8 and spec 0002 section 4. The wire adapter normalizes only
known System/Customer label-type spellings, preserving label and remote identity
conflict detection. The daemon schedules each Fleet's listener independently from
capacity reconciliation and records classified runtime failures in Fleet status
and the matching Change instead of leaving an unexplained Pending state.

Session installation and message effects compare the captured Fleet incarnation,
desired revision, mutation fence, exact observed authentication context and epoch.
The full queue handle, initial statistics and authentication pin commit together.
Clearing a session retains a monotonically advancing epoch. `Ready` with zero
demand/minimum and zero capacity is a valid idle Fleet; it is not provisioning
evidence or a claim that arbitrary future workloads will run successfully.
The public `Converged` condition additionally requires the observed revision to
match the desired revision and both effective capacity and occupancy to equal the
target derived from the current capacity policy. A Ready listener can therefore
report unconverged capacity while runners are starting or retiring.

Migration m0011 adds separate message, acquisition and observation evidence so
historical tables do not acquire incompatible deduplication constraints. Message
ingestion commits the statistics snapshot, observations, Pending acquisition
intents and wake marker before ACK. Only successful ACK advances the poll cursor;
acquisition records AcquireStarting before the request. Redelivery cannot rewind
demand or repeat the same epoch/request acquisition. Old completions retain their
original outcome even if current authentication is corrupt or replaced, without
writing new demand. Replacement statistics can reconcile old uncertain demand;
the historical evidence and independent Runner cleanup references remain.

Concurrent short SQLite transactions reserve the writer before reading a
snapshot. SeaORM 1.1.20 does not expose SQLite `BEGIN IMMEDIATE` configuration, so
`Store::begin` follows the managed begin with a no-op write before any read. This
prevents a deferred read transaction from failing its later write upgrade with
`SQLITE_BUSY_SNAPSHOT` (517), while retaining managed rollback and cancellation.
The tradeoff is serialization of transactions, including composite read snapshots;
ordinary WAL read queries remain concurrent. Network and process operations stay
outside these transactions.

Final local verification: all 525 workspace tests passed (two existing platform
tests skipped). The repository's additional `workspace test` name-filtered gate
passed 432 tests. Workspace all-target Clippy with warnings denied and rustfmt
passed. The regressions exercise real SQLite and the production HTTP/wiring path:
Pending to Ready and Change completion, ownership drift/recovery, ACK/acquisition
and redelivery, expiry/reconnect, stale epoch and repeated-stop fencing, failed
session installation cleanup, writer contention and cancellation, and independent
capacity convergence reporting. Missing/null/negative statistics are rejected.
These local checks do not establish real GitHub session acceptance or Runner
provisioning; deployment evidence is recorded separately after live verification.

Live acceptance (2026-09-09): PowerArmor `310e044` pins Shaula `c424f8e` and was
built and activated on `molecule` through Colmena. The Linux package gate passed
517 tests (two skipped), plus both HTTP-state tests including the pinned real
Terraform protocol probe. Migration `m0011_listener_messages` applied successfully.
The existing `shaula-docker-local` Fleet recovered without recreation: ownership
is Adopted on Scale Set 1, session epoch 1 has its full queue handle, observed and
desired revisions are both 1, Create Change is Succeeded, and the error is clear.
The authenticated browser also shows Ready / Observed r1 / Desired r1. Demand,
minimum and capacity are zero; this acceptance covers the live idle listener,
not a newly dispatched Actions workload or provider provisioning.

## Multi-account GitHub authentication (spec 0011 / ADR-0015): local implementation (2026-09-07)

The v1/PAT and legacy-upgrade contracts below have been retired by
[spec 0018](specs/0018-github-app-only-authentication.md) /
[ADR-0022](ard/0022-retire-legacy-github-authentication.md). Current publication,
admission and execution require v2 GitHub App policy and exact contexts.
Historical rows remain intact and visible as unsupported without decoding old
credential metadata; live/retained references require deployment preflight.
Earlier dated test results remain historical evidence, not a current support promise.

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
  permissions and installation suspension are verified. App continuity is checked
  between supported v2 revisions. Candidate rejection leaves the active
  revision untouched; transient and rate-limit failures retry with per-revision
  deadlines. Promotion atomically freezes bindings and the validation snapshot,
  rechecks all live dependencies and their mutation fences, and retargets handoffs.
- **Persistence and retention:** migrations m0008/m0009 add versioned policies,
  frozen bindings, Fleet contexts, immutable context history and session auth
  references. Historical credential bytes and replay records remain stored but
  cannot authorize or replay a removed format. Dependencies include desired and
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
  state, reasons and bindings to their exact revisions. Unsupported historical
  profiles expose only non-secret identifying metadata. Typed policy previews
  include actual live Fleet coverage; unavailable impact data blocks publication
  rather than claiming no impact. The UI has no PAT, fixed-installation or
  legacy-upgrade publication. Fleet details show the full desired/observed route
  identity and handoff status. Bounded, process-local
  runtime observations provide per-binding health; expired evidence or restart
  returns Unknown. Candidate validation is shown separately from current access.

Local verification covers real HTTP admission, the scheduled worker, SQLite
promotion and handoff transactions, supervisor-to-GitHub-adapter composition,
scripted GitHub responses, token request bodies, delayed-response races and browser
interactions. Legacy migration/replay support in earlier evidence has been retired;
current regression coverage verifies unsupported publication and authorization
rejection. Required verification commands are
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
local checks. The later Fleet listener repair above supplies session/message
composition and its durable gates. Full recovery/decommission and real-platform
acceptance remain separate: scripted-server evidence does not establish those
end-to-end workflows.

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
  safety, atomic), template publish → scan-driven static validation →
  automatic activation under spec 0017, independent conformance evidence
  (full subject verification), auth rotation with staged activation (prior active kept).
- Scale Set wire adapter (reqwest): GitHub App JWT + installation token,
  registration → Actions Service bootstrap with expiry
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
- Auth validation is scheduled by the binary. GitHub App/account/installation
  identity is checked before target access and activation. The earlier PAT
  validation matrix has been retired under spec 0018; current tests cover v2 App
  targets and unsupported-format rejection. Identity mismatch is rejected and
  transient failures leave validation pending. Endpoint contracts follow the
  official [GitHub App API](https://docs.github.com/en/rest/apps/apps).

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
  started by the binary. The listener repair above adds separately scheduled
  sessions and persist-before-ACK message ingestion/acquisition. Complete
  online/busy inventory classification, operation recovery and Fleet
  decommission/tombstone acceptance remain open. Store and scripted-listener
  tests alone do not prove these complete external workflows.
- End-to-end real-GitHub validation and the Go-oracle differential suite
  (`references/scaleset`, pinned commit) have not been executed.
- Bundled-profile conformance remains a runtime evidence gap, independent of
  automatic static activation under spec 0017. The Docker provider
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
  still required to claim that tested runtime guarantee. See the [harness instructions](docker-conformance.md).
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
| Reachable cross-Auth handoff with an idle session | Spec 0002 removes an idle session as an admission blocker while retaining the zero-Occupancy/effect barrier. The listener repair adds exact-context session gates and quiescence; real multi-account idle-session/race acceptance remains a release gate. |
| Profile retirement self-head release | `service_profile_retirement.rs` and `shaula-store/src/registry_impl/retirement.rs` retain current heads. Spec 0005 §7.1's final self-reference release/Retired transaction and history-versus-runtime-reference tests are missing. |
| Binding reads | Existing core/Profile reads expose coarse `bindings_present`; spec 0005 §5.2 retains that conservative projection, not a new per-secret fingerprint map. Full manifest annotation/schema/redaction acceptance still needs verification. |
| Attestation integrity/evidence | Existing `service_profile_attestation.rs`, `service_attestation.rs` and core `registry/attestation_subject.rs` provide authority/subject handling. No independent signing PKI is required by the clarified contract; complete external report linkage, canonical compatibility and real-platform suite evidence are not established by local record-acceptance tests. |
| `bindings_digest` compatibility | Current `BindingsDigest::from_keyed_material` in `crates/shaula-core/src/template.rs` emits `bd1_` HMAC-SHA256; its material does not include Profile incarnation. D4 remains deferred. No new encoding or historical-record rewrite was made. |

## Mandatory management OIDC: implementation and evidence

- Provider-backed browser session renewal (2026-09-08, spec 0013 / ADR-0017) is
  implemented in `shaula-http/src/oidc/`: the guard renews stale leases before
  admission, keeps refresh tokens only in bounded daemon memory and preserves
  session ID/CSRF. Provider lifetime governs continued renewal; short expiry and
  idle require fresh proof. Logout/replacement fence in-flight completion.
  `token_exchange.rs` shares the code/refresh exchange and absolute expiry logic;
  `renewal.rs` and `session_refresh.rs` own cancellation-safe singleflight.
  Regression entrypoints are `shaula/tests/oidc_renewal.rs` (admission, concurrent
  requests, cancellation, logout/replacement, outage/backoff),
  `shaula/tests/oidc_refresh_tokens.rs` (optional/rotated tokens, identity and scope
  binding, malformed/oversized responses, JWKS failure and finite fallback), and
  internal session/expiry tests. `web/e2e/renewal.spec.ts` uses the real daemon
  behind HTTPS with a test Provider; it checks URL/hash, the same mounted draft
  input, stable cookie/CSRF, token rotation and a mutation sent once. No test
  Provider controls are included in the production router. Actual registered
  Provider expiry/renewal acceptance must be recorded separately from this fixture.
  Local validation on 2026-09-08 passed rustfmt, strict workspace/all-target Clippy,
  417 unfiltered workspace nextest tests (two existing ignored tests), 32 frontend
  tests, frontend formatting/build/lint, and both real-daemon HTTPS browser tests.
  The repository's additional `nextest ... workspace test` name-filtered command
  passed 337 tests; it is not used as a substitute for the full workspace gate.

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

- Automatic Template activation: [spec 0017](specs/0017-automatic-template-activation.md)
  and [ARD-0021](ard/0021-activate-templates-after-static-validation.md) are implemented
  (2026-09-08), after independent specification and implementation reviews.
  The normal scan revalidates current Validating/Ready candidates and atomically
  commits the Active head, revision state, immutable activation audit, outbox and
  matching Publish Change. Incarnation, desired revision, artifact and retirement
  checks reject stale results. Existing Ready revisions need no republish; repeated
  scans and historical Active revisions preserve their activation identity.
  Conformance submission retains exact-subject verification, immutable evidence
  and replay semantics but never changes activation. Historical attestation field
  names carry opaque activation provenance; no conformance rows are fabricated.
  Templates explains automatic activation and shows bounded validation reasons;
  existing raw parser reasons are sanitized before revision reads.
  Final local validation passed: 477 unfiltered workspace tests (2 skipped),
  387 tests in the literal AGENTS filtered command, strict workspace/all-target
  Clippy, rustfmt, frontend build/lint/format, 72 browser tests and 4 real HTTPS/OIDC
  browser tests. Store tests cover transaction rollback/retry, stale success and
  rejection fences, prior Active retention, invalid lock and recoverable reads.
  HTTP tests prove publication with only `template.publish` automatically activates,
  Fleet admission needs no attestation, and r2 promotion preserves existing r1 pins.
  The HTTPS fixture starts with a persisted Ready revision and no activation ID,
  then verifies real daemon activation, server selection and approved input loading.
  These checks establish automatic activation and admission, not runner provisioning
  or complete platform conformance. Deployment is managed by PowerArmor's locked input.

- Stored Template library: [spec 0015](specs/0015-template-library-and-variable-discovery.md)
  and [ADR-0019](ard/0019-store-template-sources-and-discover-terraform-variables.md)
  are implemented (2026-09-08). Migration m0010 stores immutable original archives
  and default source selections in SQLite. Startup imports legacy sidecars without
  repacking, restores missing execution material, and seeds each configured source
  key once. Source removal or a package update cannot replace that selection or
  publish/activate a Profile. Runtime reads and Create/Destroy also verify cache
  material against database authority. Missing legacy authority and changed cache
  contents remain explicit errors. Default packaging excludes image build context,
  state, tfvars and unrelated files; the Nix package/module installs and imports
  the two bundled source modules.
  The `template.read` source/variable endpoints return private, bounded discovery
  attributed to the exact archive. HCL AST parsing discovers typed `shaula.bindings`
  and `shaula.parameters`, literal optional defaults and schema descriptions/options;
  unsupported expressions and schema disagreement are explicit. Sensitive subtrees
  suppress defaults/options, and existing untyped `shaula = any` templates keep their
  prior runtime behavior with discovery unavailable. Bundled defaults now live in
  Terraform declarations, without duplicate manifest defaults.
  Templates lists default sources separately from published Profiles. Selecting a
  source fixes its digest in the publication draft and exposes scalar binding editors
  and variable descriptions in the main form. Adopting defaults or options requires
  an explicit action and preserves existing drafts; inspection never grants Fleet
  input permissions or activates a Profile. Archive inspection reuses the upload.
  Validation entrypoints include artifact library/store tests, variable parser and
  constraints tests, HTTP read authorization/error tests, `template-library.spec.ts`
  and the real HTTPS/OIDC `library.spec.ts`. Final local validation passed: 465
  unfiltered workspace tests (2 skipped), the literal AGENTS filtered run (379
  passed), strict all-target Clippy, rustfmt, frontend build/lint/format and 58
  browser tests, plus 4 real HTTPS/OIDC browser tests against the final implementation.
  Terraform 1.9.8 validated both complete bundled modules in isolated
  Windows copies with temporary platform-specific provider locks; the repository
  locks were unchanged. Separate resource-free plans checked omitted and explicit
  null input defaults for both modules. Independent storage and variable reviews
  found issues in error classification, source filtering/removal, numeric fidelity,
  sensitive inheritance and nested null; fixes and regression coverage were verified.
  The production Nix package also passed 457 Linux workspace tests (2 skipped),
  strict Clippy and both real Terraform HTTP-state protocol checks. The NixOS
  module evaluation and full treefmt gate passed; the existing Docker conformance
  and bootstrap Python suites passed 29 and 11 tests. The custom Docker smoke
  runner label is declared in `.github/actionlint.yaml` and explicitly loaded by
  actionlint in Nix source archives. Production deployment is managed by
  PowerArmor's locked `shaula` input.

- Fleet Profile selection: [spec 0016](specs/0016-fleet-profile-selection.md)
  and [ARD-0020](ard/0020-load-fleet-profile-choices-from-registry.md) are implemented
  (2026-09-08), following independent specification review and a separate final
  implementation review. Both fields read the existing server collections and
  present sorted, unselected dropdowns with Active revision/status, per-list refresh,
  and explicit loading, permission, empty, unavailable and failure states. New
  references require a successful list and an eligible current choice; an older
  Active remains selectable while its desired candidate is validating.
  Selecting a Template immediately loads its exact Active input contract. List
  refreshes preserve captured versions, input drafts and the Fleet write version.
  Unchanged original references survive missing permissions or list failures, and
  explicit restore actions return to the original authentication or Template/inputs.
  Template collection reads now propagate stored-row faults instead of returning
  a partial successful list, with real HTTP/SQLite regression coverage.
  Final local verification passed: 468 unfiltered workspace tests (2 skipped),
  379 tests in the literal AGENTS filtered command, strict workspace/all-target
  Clippy, rustfmt, frontend build/lint/format, 70 browser tests and 4 real
  HTTPS/OIDC browser tests. The 390px dialog screenshot was visually inspected.
  The HTTPS fixture selects both real collection entries, loads approved inputs
  automatically, and verifies the exact PUT plus preserved drafts after a local
  target-policy rejection; it does not create a runner or call GitHub. This
  increment was deployed through PowerArmor's locked Shaula input at `0dd546e`.

- Visual Fleet Template inputs: [spec 0014](specs/0014-visual-template-inputs.md)
  and [ADR-0018](ard/0018-render-fleet-inputs-from-approved-template-options.md)
  are implemented (2026-09-08). The exact-revision input-contract read projects
  approved fields or complete presets through shared admission validation, with
  bounded output, `template.read`, private/no-store and explicit unavailable errors.
  All Template inputs, including optional controls and presets, appear directly
  in the main Fleet form. Advanced settings contains only other Fleet settings.
  Values retain their JSON types and integer precision without editable raw JSON
  or implicit defaults. Existing inputs and bare references survive failed reads;
  explicit template changes capture an exact revision and require draft discard
  confirmation when needed. Background reads cannot replace the captured contract.
  Validation passed: frontend build, lint/format and 51 browser tests (including
  16 new visual-input regressions); 3 real HTTPS/OIDC browser tests; strict workspace
  Clippy, rustfmt and 424 workspace tests (2 ignored). The literal AGENTS nextest
  name-filter command also passed (341 tests); the unfiltered run is the full gate.
  The HTTPS input test uses an isolated seeded Template read fixture, exercises
  the real daemon projection/editor and verifies the exact PUT plus an expected
  local admission rejection without creating a Fleet. Router/SQLite integration tests
  separately cover successful admission and preset rejection. These checks do not
  claim real template activation or runner provisioning. Independent frontend and
  backend review found no blocking defects.

- Creation dialogs (2026-09-08) show required basics first and keep other optional
  Fleet, Template and GitHub App settings in a shared collapsed Advanced settings
  section. Hidden values stay mounted; native validation reveals invalid controls,
  and JSON errors open the relevant settings. Fleet creation defaults scale set
  name to its key, template source selection retains archive/digest drafts, and
  additional App targets retain their explicit policy when folded. Existing
  policy impact previews and mutation version/idempotency checks remain visible
  or enforced as before. The contract is in spec 0008; regression entrypoints
  are `web/tests/advanced-forms.spec.ts`, `advanced-auth.spec.ts` and the existing
  mutation, policy-upgrade and conflict suites. Validation passed: frontend build,
  lint and 35 browser tests; 2 real HTTPS/OIDC browser tests; strict workspace
  Clippy, rustfmt and 417 workspace tests (2 ignored).

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
  no frontend directory. Setup and verification commands are in [the development guide](development.md).
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
