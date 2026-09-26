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

## Explainable reconciliation diagnostics (2026-09-26)

PR #4's spec/ARD 0041 is merged. The local implementation adds a closed 41-code
catalog, exact decimal capacity evidence, optional bounded decision capture,
forward migration m0026, three `fleet.read` GET endpoints, typed Rust client,
CLI explain/table/JSON/watch and Fleet/Generation/Job detail-page Why panels.
Each query preserves capture time and version/identity fences. Projection loss
returns incomplete explanations; authoritative-store failure remains 503.

The sink has 1,024 queue entries, 64 KiB records, finite lanes, work-start
sequences, process epochs and independent SQLite transactions. Dropped or failed
writes invalidate currentness and later observations can recover. Reads do not
call backends, select pool members or consume routing randomness. Reports retain
at most 64 reasons, 128 evidence entries and 32 related identities within
256 KiB, preserving primary causes and references. A separate periodic collector
removes only disposable projection rows, including records past seven days or
whose authoritative subject has been removed.

Producer/predicate ownership:

| Evidence | Implementation |
| --- | --- |
| Missing/stale/conflict/clock-invalid/unclassified | core diagnostics, Store hub/writer/read and daemon capture completion |
| Auth/dependency/ownership/listener/demand/inventory/rate-limit/decommission | GitHub and Forgejo supervisor plus both assembly paths |
| Target/policy ceiling/occupancy | original capacity input tuple in each supervisor |
| Pool cap/no eligible/inline backpressure/selection | original pool admission transaction; protected member details filtered at read |
| Create/destroy slot wait | actual semaphore acquisition path |
| Apply outcome/bootstrap/readiness/operation result | typed runtime results, retained operation ledger and both readiness drivers |
| Busy/safe-drain/registration/quarantine/hard-lifetime/completed | shared RunnerOperation plus ledger checkpoints; completion source distinguishes never-started, provider cleanup and operator attestation |
| Follow resolution/inputs/backend/occupancy/commit | original single-template and shared-pool follow branches; pins require template.read |
| Job association/dispatch boundary | retained backend-specific Jobs facts; no invented Generation association or same-name Fleet link |

Local evidence includes real SQLite/Axum/OIDC authorization and fault tests,
work-start ordering/epoch replacement/conflict/time expiry, write failure and
recovery, a 100,000-Generation aggregate fixture, bounded report/reference tests,
and identical lifecycle effect traces with diagnostics disabled/enabled/failed
for both Runner Backends. Browser and CLI tests cover display and compatibility.
These are local tests, not real provider acceptance.

Verified locally on Windows:

- `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` passed.
- Unfiltered `cargo nextest run --manifest-path Cargo.toml --workspace --no-fail-fast --status-level fail --final-status-level fail`: 1,004 passed, 2 skipped.
- The separately requested `cargo nextest run --manifest-path Cargo.toml --workspace test` uses a name filter; it is supplementary to the unfiltered suite.
- Web `npm run lint`, `npm run fmt:check`, `npm run build`, and the complete Playwright suite passed (202 tests); all 8 diagnostics browser tests were repeated after the final display adjustment.
- All changed Rust files are at most 400 lines. Rust and Web catalogs both contain the same 41 reason codes.

**Release gate still open: DX-30.** A selected isolated GitHub/Forgejo repository
and resource platform/version tuple are required for real no-create,
waiting-online, destroy-failure and rollout-lag scenarios. No production failure
injection or deployment was performed. No claim is made that local mocks cover
external platform behavior or all six resource platforms. Test totals above do not replace that release gate.

## Personal access tokens and remote CLI (2026-09-25)

PR #3's spec/ARD 0039 is merged. Local implementation now includes owner-bound
opaque PAT issuance/verification/revocation, finite TTL/quota/rate limits,
primary-OIDC-only delegation, credential audit provenance, and principal plus
HTTP-operation idempotency namespaces. Migrations m0023–m0025 preserve unknown
legacy ownership as a conflict. Existing OIDC browser/startup requirements stay
in place. Token secrets are returned only on the first successful issue response.

`shaula-api-types` and `shaula-client` provide independent transport contracts
and all 49 inventory operations. The remote CLI covers Fleet, Template, Pool,
GitHub/Forgejo Auth Profile, history/log, Change and token commands. Strong
versions, immutable attempts, exact numeric request tokens, separate finalize
receipts, bounded reads, no redirects and private credential files are implemented.
The Web token page supports explicit scopes, ephemeral secret reveal, recovery,
verified replacement followed by optional revocation, and disabled deployment
policy. Usage and recovery procedures: [CLI](cli.md), [deployment](oidc-deployment.md).

Local evidence:

- Full workspace nextest: **955 passed, 2 skipped**. The two ignored tests are
  opt-in harnesses, not silently counted as passing. The prescribed trailing
  `test` command also passed (740 passed, 217 skipped), but is a name filter.
- Real Axum/OIDC/SQLite tests cover issue/replay, same owner across credentials,
  different owners, legacy conflicts, operation/precondition namespaces, audit,
  revocation, quota races, policy changes and all 49 route authorization boundaries.
  Bidirectional router/inventory checks and SDK routing tests reject route drift.
- CLI subprocess workflows cover GitHub/Forgejo publication and rotation, write-only
  access, Fleet/Pool create/update/conflict/retire, Template typed projections and
  omitted/null Update semantics, context/login/logout, stream cursors and partial
  results. These exercise production handlers and store code; external provider
  activation/Runner execution is fixture-driven in these management tests.
- A production `shaula serve` subprocess test proves persisted PAT use after
  restart and during a runtime Provider outage, mandatory Discovery on startup,
  and rejection after disabling PATs or removing the owner's grant on restart.
- Web regression: **194 passed**; all four token-page tests pass. Rustfmt, strict
  all-target clippy, standalone SDK build, Web lint/build and full Web formatting
  pass. Explicit LF attributes fix Windows checkout differences in template
  integrity inputs and formatter-controlled Web sources. The separate real-daemon
  HTTPS/OIDC browser suite passes **9 tests**, including:
  browser login and issuance → CLI Fleet read → self-revocation → CLI 401 → browser
  revoked metadata. It uses the local test Provider, not the deployed identity service.

Deployment: molecule's complete system update passed Linux Clippy, 949 nextest
tests (2 skipped), and 2 Terraform HTTP-state tests, then activated successfully.
The actual HTTPS Browser session recovered Indexyz's identity and displays the
new Access tokens page. Migrations m0020–m0025 and live/backup integrity checks
passed; the full stopped-service state backup is retained. Source snapshot,
system paths and logs are recorded in [deployment evidence](evidence/0039-deployment-2026-09-25.md).

Actual deployment acceptance passed: browser-primary issuance, CLI PAT identity
and Fleet reads, HTTP 403 for an undelegated scope, continued PAT use after a
service restart, self-revocation and HTTP 401 on reuse. Browser and SQLite confirm
revocation; zero unrevoked PATs remain after the test.

Remaining release acceptance: actual Provider-outage and backup-restore exercises
remain open. The deployed
Kanidm registration cannot mint the distinct primary API-JWT audience. Exhaustive
paired Web/CLI evidence for every WF-01–WF-14 failure branch is not claimed by the
route coverage or existing component suite. Spec 0039's complete external/parity
acceptance remains open until those results are recorded.
## Shared conditional publication (C2, 2026-09-25)

Fleet, Template Pool, Template Profile and both GitHub/Forgejo Auth PUT now enter
one internal daemon Module (`conditional_put/`). Existing Registry Interfaces
remain the callers' test surface. Typed resource Adapters retain canonical bytes,
exact-pin/admission rules, protected-material comparison and transactional fences;
Template Update keeps its preparation before entering Template publication.

- Pool no-op now checks the exact live head, audits and saves an optional durable
  `200` replay in one writer transaction, with no Revision/Change/outbox growth.
- CAS/commit losers with an idempotency key perform a read-only replay recheck.
  They never change the request ETag, re-admit or retry a write.
- New mixed `If-None-Match: *` + `If-Match` PUTs return
  `400 ConflictingPreconditions` after authorization, format and replay checks.
  Matching history still replays; key/content conflicts remain `409`.

Local verification: **16 new HTTP/Registry + real SQLite tests** pass, including
five frozen historical hash fixtures, all five publication variants, bounded
concurrency barriers, protected-material conflicts and transactional fault
injection. Disabling the shared replay recheck caused three concurrency tests to
fail; restoring it made them pass. Full workspace nextest: **952 passed, 2 skipped**.
The required name-filtered `cargo nextest run --manifest-path Cargo.toml --workspace
test` passes **749 tests, 205 filtered/skipped**. Rustfmt, strict workspace/all-target
Clippy and whitespace checks pass; every C2-touched/new Rust file is <=400 lines.

No schema migration, hash rewrite, public Registry parameter change, unified
error channel or MutationFacts redesign was introduced. Read models and SQLite
read projections were physically split only to respect the file-size rule.
DELETE, Policy Update, Finalize, attestation and cascade orchestration stay intact.
No real-platform or remote-CI acceptance was run for C2.

Deferred, not silently resolved: identical full Auth PUT still creates a Candidate
(the spec 0005 no-op discrepancy); principal/method idempotency scope still needs
compatibility design; Template no-op versus Retirement remains an unverified race
lead. C1 Create/restart/admission and C3/C5 are separate work.

## Runner Operation: shared Destroy for both Runner Backends (2026-09-25)

The GitHub Retirement Destroy, the Forgejo Destroy and the Runner Maximum Lifetime
reaper were three copies of the same Generation effect; they are now one daemon Module
(`crates/shaula-daemon/src/runner_operation`). Drivers keep candidate selection; the
Module owns the state walk, destroy proofs and Quarantine rules. The Runner Backend
seam holds only registration admission, removal and unrecorded-absence proof.

Behavior changes (intended, see [ARD-0040](ard/0040-destroy-generations-that-never-started-a-create-apply.md)
and spec 0024 §2.1 rev 4):

- A Generation that never admitted a Create apply and whose registration is proven
  removed or absent becomes `Destroyed` without Terraform on both backends (GitHub
  previously quarantined it after 60 s and kept the capacity slot).
- GitHub Destroy now quarantines missing bindings or an invalid manifest instead of
  aborting the whole pass or destroying with an empty managed shape.
- One Generation's failure no longer aborts the rest of a pass; the separate
  `quarantine_stale_cleanup` step is removed.
- An expired Generation without a registration identity is quarantined after its
  resources are destroyed instead of retrying forever.

Verification: `cargo nextest run --workspace` 936 passed (14 new Runner Operation matrix
tests run each scenario against both adapters); `cargo clippy --workspace --all-targets
-D warnings`; `cargo fmt --check`. Create, restart classification and the admission
transaction (the remaining C1 steps) are unchanged; no real-platform run was performed.

## Forgejo management, real lifecycle and Jobs (2026-09-22)

The three usable-version increments are implemented:

1. Forgejo Fleet PUT originally admitted only a single Template Profile; the weighted
   follow-up below adds inline/shared TemplatePool support. Web types, mixed Fleet list/detail,
   creation/editing and authentication now branch by backend. The shared Auth API
   path remains `/github-auth-profiles`; Forgejo token publication/rotation uses its
   own wire fields, with target/impact/validation shown without returning the token.
   Template discovery exposes the **Active revision's selected** `runnerBackend`,
   not the candidate's capabilities. Unknown selection is null. Existing GitHub
   drafts and retained pins preserve their prior behavior.
2. `scripts/forgejo-lifecycle` runs actual `shaula serve`, HTTP/OIDC admission,
   SQLite, Terraform 1.9.8/provider 3.0.2, real Docker Engine 29.6.2 and Forgejo
   16.0.4/Runner 13.1.0. Success/failure, Busy restart, idle/Busy hard expiry and
   two independent DELETE failure checkpoints converge through the production
   runtime. Assertions require one Generation, Destroyed, zero occupancy, and
   exact registration/container/anonymous-volume absence. CI now has a separate
   lifecycle job; local passing evidence does not claim the remote CI job ran.
   This exposed and fixed a real periodic-scan race: the structural scanner was
   rejecting every Forgejo Candidate as malformed GitHub auth before its online
   worker. HTTP+SQLite red/green regression now protects the provider distinction.
3. Forgejo Jobs snapshots persist separately from GitHub message/assignment tables
   (forward migration m0021), sharing only the bounded list/detail read surface.
   Scope/Fleet incarnation/Auth Profile/repository/job/attempt namespaces and u64
   string IDs prevent accidental merging. Waiting/running are direct observations;
   disappearance is Unknown with retained last status, never invented completion.
   Read-time 30s staleness, failed-poll timestamps, change-only events, pagination,
   retention, and unverified/no-Generation associations have local regression
   coverage. Exact Forgejo runner IDs have their own Runner read field.

The focused simplification/Rust review fixes are included:
- **S1:** Jobs history failures no longer gate validated demand or ordinary Runner
  reclamation. Independent bounded/nonzero/deduplicated demand identities and the
  current Fleet guard remain mandatory. History failure reports a finite reason,
  marks stale best-effort, and retains read-time expiry when the marker also fails.
  Four SQLite/supervisor regressions cover completion cleanup, rejected display
  metadata, invalid identities, and simultaneous poll/history failure.
- **S2:** Both Auth forms share the impact query and readiness gate. Two browser
  regressions prove malformed impact blocks button and programmatic submission,
  then recovery preserves the draft credential and permits exactly one write.
- **S3:** Removed the unused optional target-filter argument and scope comparator
  from backend Auth choices; both consumers retain provider filtering.
- **S4:** Removed the fixture's write-only global volume set and unused server
  inspection. Exact observed Runner-volume absence assertions and owned-resource
  teardown remain; no cleanup or trust boundary was removed.

Verification: `cargo fmt`, strict workspace/all-target Clippy; complete nextest
**893 passed, 2 platform skips**; required `cargo nextest run --manifest-path
Cargo.toml --workspace test` **708 passed, 187 filtered/skipped**. Web build/lint,
format and **183 Playwright tests** passed. Twelve new browser cases cover Forgejo
management/Jobs and shared Auth impact recovery. Earlier desktop/mobile screenshots
were inspected in bounded passes and the UI detector reported no findings. All 31
changed/new Rust files are <=400 lines. The Vite bundle-size advisory (>500 kB)
remains non-fatal and is not addressed by this work.

Real acceptance was rerun after S1–S4 with the Jobs projection enabled: all eight
checks pass. See [sanitized lifecycle evidence](evidence/forgejo-lifecycle-2026-09-21/README.md)
and [operator setup](forgejo-templates.md). Local rootless DNS required an official
provider filesystem mirror with readonly lock validation; no mock engine or
modified template was substituted. Private raw state/config/logs are not committed.
Final `cargo fmt --check`, web format/lint, script syntax/fault tests, workflow
`actionlint` and `git diff --check` pass. The isolated Docker Engine had no remaining
containers or volumes after acceptance and was stopped; system Podman was untouched.

Weighted TemplatePool follow-up (2026-09-22): Forgejo now reuses common transactional
member selection/caps, persists exact member inputs, and preserves the selection on
restart. HTTP admission rejects incompatible backend/label targets; pool/profile
retirement is rechecked at routing commit. Capacity-only PUTs retain old inline/shared
pins instead of duplicating hydrated shared members or requiring old pins to remain
Active. The existing Fleet form offers single, weighted and shared placement.
Local verification: strict workspace Clippy, 909 workspace tests (2 skipped), Web
lint/build and 13 Forgejo management browser tests passed; desktop/mobile placement
screens were inspected. These are not real Kubernetes/VM or weighted-load acceptance.

Further follow-up (2026-09-22):

- **Inputs commit fence:** changed `template_inputs` require zero Generation/open-effect
  occupancy inside the same SQLite writer transaction as Fleet PUT. Unchanged inputs
  still permit capacity-only updates. Races use the public Store commit boundary.
- **Profile retirement:** periodic scans recheck live Fleet/pool/Generation references,
  operations, worker leases, HTTP-state capabilities/locks and auth execution contexts.
  Once quiescent, Template/Auth release their active/observed heads and become Retired,
  completing the Retire Change atomically. Repeated DELETE stays terminal. History and
  protected credential/artifact bytes are retained, not force-deleted or garbage-collected.
- **Exact Jobs results:** forward migration m0022 persists optional history budgets.
  Independent bounded reads join the previously observed repository/Task ID to repository
  task history, not a workflow summary or Runner. Results/events commit atomically under
  current Fleet/scope/task fences; absence alone remains Unknown. Old JSON is readable,
  budgets survive restart and stale snapshots cannot erase results or move updated time
  backwards. See [Jobs limits](forgejo-jobs.md).
- **Real acceptance:** [Docker and Kubernetes receipts](evidence/forgejo-followup-2026-09-22/README.md)
  pass 13 and 10 checks respectively: normal completion/failure, restart, labels, stale
  demand, hard expiry, exact Jobs results and lost-registration quarantine. Docker also
  exercises DELETE retries and the [four-scope permission matrix](forgejo-permissions.md).
  Local Kind 0.32.0 / Kubernetes v1.36.1 uses the existing template and pinned provider.
- **Kubernetes correction:** kubectl discovery cache now goes to a private temporary
  directory for both backends, not frozen Workspace material. A failing real Destroy and
  red/green unit test exposed the issue; the final real rerun proves convergence without
  weakening material integrity. Real labels also corrected the old reversed subset copy.
- **Approved credential boundary:** Kubernetes provider refresh copies the single Runner
  token into protected Destroy plan/state/backup Secret data. The operator explicitly
  accepted this; actual owner-only permissions and absence from inputs, metadata, argv/env
  and log/API projections are checked. The passing receipt records two retained token
  occurrences, rather than claiming zero. Management credentials are never permitted there.

Final local verification for this follow-up: `cargo fmt` / `cargo fmt --check` and
`cargo clippy --workspace --all-targets -- -D warnings` pass. The unfiltered workspace
nextest run passes **922 tests, 2 skipped**; the required `cargo nextest run
--manifest-path Cargo.toml --workspace test` passes **735 tests, 189 filtered/skipped**.
Web format/lint/build and **190 Playwright tests** pass, including 19 Forgejo management
and Jobs cases. The non-fatal Vite bundle-size advisory remains. Desktop/mobile views
were inspected and both focused UI detector runs reported no findings. All 54 changed/new
Rust files are <=400 lines. All four lifecycle fault/leak helper tests, script syntax,
workflow `actionlint` and `git diff --check` pass. No remote CI or production deployment
is implied. The isolated Kind cluster was deleted; the empty private Docker Engine was
stopped, temporary Nix roots removed, and host inotify watches restored to 524288.

Still not established: safe early idle acquisition fencing, real VM/cloud and GitHub
acceptance, the full real A5 None/ExactlyOne/Multiple matrix, weighted-load distribution,
or the minimum-version/all-deployment permission matrix. The real lost-response case
correctly quarantines an undeclared registration; fixture teardown is not automatic reclaim.
The explicit hard lifetime may interrupt Busy jobs. No acquisition proxy, `--handle`
routing, Verified Forgejo association or workflow cancellation UI was introduced.
Earlier Forgejo sections are historical and are superseded by the increments above.

## Forgejo selection on existing VM templates (2026-09-21)

Proxmox, AWS, TencentCloud and AliCloud now expose the same immutable publisher
`bindings.runner_backend` choice as Docker/Kubernetes, defaulting to GitHub.
No new TemplatePlatform/source kind or acquisition proxy is introduced.
`shaula.forgejo-vm-cloud-init/v1` explicitly admits only the pre-registered
**per-Runner** token in protected `shaula.forgejo_vm.token` and cloud-init;
GitHub/container envelopes reject this field. The supervisor already registers
with server-enforced `ephemeral: true`; guests never register a second time.
Management/provider credentials remain outside guest materials. See
[VM setup and credential boundaries](forgejo-templates.md).

VM bootstrap pins the official native Forgejo Runner 13.1.0 amd64 release and
its SHA-256, validates the download, consumes a durable one-start marker and
launches `one-job --wait` as non-root with a private token file, not argv/env.
The systemd unit has no boot enablement, Restart=no and NoNewPrivileges=true.
Proxmox also requires root-only, unmounted CIDATA before launch. Existing VM
GitHub bootstrap files and resource ownership shapes are unchanged. Forgejo
completion still requires exact registration evidence before resource cleanup;
never-assigned/overlong Runners are covered by the shared 7200-second hard
lifetime. Ephemeral does not delete the VM or erase Terraform/user-data copies.

Verification: rustfmt and strict workspace/all-target Clippy pass. Required
`cargo nextest run --manifest-path Cargo.toml --workspace test` passes 698 tests
(186 filtered/skipped, 4 threads); the final unfiltered run passes 882 tests
(2 platform skips, 2 threads, no-fail-fast). New regressions cover VM-only token
serialization/redaction/admission, backend discovery and all four VM lifecycle
fixtures through exact ephemeral completion and resource reclaim. All changed
Rust modules remain within 400 lines; all six runtime policy digests match.

Terraform 1.9.8 readonly locked init/validate passes for Proxmox 0.5.1, AWS 5.67.0,
TencentCloud 1.83.31 and AliCloud 1.292.0 (Tencent registry checksum download
initially returned HTTP 504; unchanged retry succeeded). The reproducible
`scripts/template-backends/vm.mjs` evaluates the actual user-data expressions
without provider/resource blocks: all eight platform/backend renders pass,
plus 19 guest fixture cases with real Bash/Python, checksums, file modes and
replay/failure gates. OS identity, downloads/packages and block devices are
explicit test substitutes, not real cloud-init/systemd or cloud acceptance.

An initial full 4-thread run hit the previously observed shared OIDC discovery
`Provider` error in `follow_cascade::changed_inputs_validate_against_the_active_revision_schema`.
The exact isolated test and final full run pass without OIDC code changes;
this is recorded as a test-stability risk, not a proven OIDC fix. No real cloud
resources were created. Target-environment Forgejo boot/job/VM-disk-ISO cleanup,
Fleet UI/Jobs integration and ordinary idle-safe drain remain separate gates.

## Backend selection on existing container templates (2026-09-21)

Docker and Kubernetes now advertise `runner_backends: [github, forgejo]` on the
same bundled sources/platforms. The publisher selects `bindings.runner_backend`
(default GitHub), frozen in the Template Revision; Fleet parameters cannot
switch it. `runner_image: auto` selects the corresponding official pin. The
Forgejo 13.1.0 OCI index is pinned to
`sha256:c4af85fd9f0dd03788676a534781a87c71aa2c6a37737143e017eb94d4312952`
(its registry index lists linux amd64 and arm64). Publication, Fleet admission,
single/shared-pool follow and both Create paths check the selected backend;
mismatches do not mint JIT/register a runner or silently move a Fleet's pin.
Historical single-backend manifests keep their original meaning. See
[publication examples and limits](forgejo-templates.md).

The templates select the existing official GitHub or Forgejo `one-job --wait`
bootstrap. Real Terraform 1.9.8 plans exposed provider representation details
not covered by the original hand-built fixtures: Docker 3.0.2's empty env and
Kubernetes 2.33.0's empty Secret data become unknown, and the Kubernetes UID
field is a string. Fixed nonsecret backend markers keep launch gates known;
unknown controls and prepopulated token keys remain rejected. No proxy or
new VM bootstrap was added.

Verification: strict workspace/all-target Clippy, rustfmt, Terraform fmt,
readonly locked init/validate, and all four saved-plan admission checks pass.
`scripts/template-backends/plan.mjs` reproduces the plans using real providers
against read-only local API fixtures, without apply or real resources. Targeted
Rust checks pass 268 tests. Final nextest passes 690 `test`-filtered tests
(186 filtered/skipped), and 874 unfiltered workspace tests (2 platform skips),
with 4 test threads and the saved-plan oracles enabled for the full run. An
initial 8-thread run hit an existing OIDC fixture discovery `Provider` error;
that exact test passed in isolation and both final runs passed without OIDC
code changes. All changed Rust modules stay within 400 lines.

These are local contract/plan tests, not real Forgejo + Rust Pool + Terraform
end-to-end or Kubernetes cluster acceptance. Forgejo Fleet UI/Jobs projection,
busy-safe idle drain and the remaining A1–A8 gates remain open.

## Shared Runner hard lifetime (2026-09-21)

GitHub and Forgejo now share the server bootstrap setting
`runner.max_lifetime_secs` (default 7200). It counts from successful Create,
not job start or allocation, and intentionally terminates running jobs at
expiry. `m0020_runner_lifetime` persists the success timestamp, expiry intent
and resource-destruction checkpoint. Hard cleanup destroys original pinned
resources first, then retries exact-authority remote deregistration; GitHub
`JobStillRunning` cannot indefinitely keep the resource alive. Occupancy is
released only after both parts complete. Demand/inventory outages and handoff
readiness do not gate hard cleanup on an already constructed supervisor;
missing ownership still fails closed. See [configuration and upgrade risks](runner-lifetime.md).

Local SQLite/port tests cover deadline boundaries, Busy expiry, policy overrides,
slow Create completion, restart/resume with a longer policy, API/Destroy failures,
registration conflicts, missing ownership, stale Fleet fences and migration.
Verification: `cargo fmt --all -- --check` and strict workspace/all-target Clippy
pass. The AGENTS `test`-filtered nextest run passes 679 tests (184 filtered/skipped);
the unfiltered workspace run passes 861 tests (2 platform skips), both with 8 test
threads. Existing migration snapshots/counts were updated for schema 20, and a
new failure-injection test verifies atomic migration rollback/retry. One OIDC
fixture discovery failed during the first unconstrained run; its isolated retry
and both final workspace runs pass without OIDC code changes.

This does not prove real-platform hard-timeout acceptance or close Forgejo A1–A8.
Production Forgejo retains official `one-job --wait`; no acquisition proxy is added.

## Tencent Cloud and Alibaba Cloud runner templates (2026-09-17)

The bundled catalog now includes `tencentcloud` and `alicloud`. Each Template
creates exactly one pay-as-you-go CVM/ECS instance from a publisher-bound Ubuntu
22.04 x86_64 image and existing network/security groups, delivers JIT through
cloud-init, verifies the pinned GitHub Actions runner archive, and destroys the
instance through the existing safe-removal lifecycle. Provider credentials remain
publisher-only bindings and are absent from guest materials. Both VM-image
contracts explicitly trust the bound image ID rather than claiming an OCI content
pin. Platform metric labels, default-source synchronization and Nix packaging are
wired for both stable source keys.

Terraform 1.9.8 initialized the pinned `tencentcloudstack/tencentcloud` 1.83.31
and `aliyun/alicloud` 1.292.0 providers, generated locks containing the Linux
AMD64 `h1:` checksums, and validated both root modules without warnings. Local
manifest, variable-discovery, credential-separation and publication regressions
pass. Strict workspace Clippy and rustfmt passed; the unfiltered workspace run
passed 837 tests with 2 platform skips, and the literal AGENTS filtered run passed
662 tests with 177 skips. A clean path-based Nix package build passed its own
checks and installed all six default source directories with complete cloud
artifacts. These are static/local checks: no Tencent Cloud or Alibaba Cloud
resource was created. Real image/cloud-init compatibility, KMS permissions, JIT/job
execution, busy-safe removal and final instance/system-disk cleanup remain target-
account acceptance gates. See [the operator guide](cloud-runners.md) and each
artifact's `runtime-policy.md`; this increment does not claim production readiness.

## Job observations without a request identity (2026-09-09)

The Ready observation after deploying `35cf06f` was temporary. The user reported
`AccessVerificationFailed` again, and the next production investigation isolated
a different failure after successful access and labels verification. Queue message
`100000001` contained `totalAssignedJobs=1` and one `JobAssigned` with a nonempty
job ID, `runnerRequestId=0`, and no Runner ID/name. The store rejected it as
`invalid observed runner request ID`, rolled back demand and observations, and
left the ACK checkpoint at zero. The same message was repeatedly delivered; the
generic listener failure surfaced as `AccessVerificationFailed`.

Spec 0001 §8, spec 0019 §2 and ARDs 0003/0023 now distinguish zero request
Assigned/Started/Completed observations from positive acquisition/request identity.
The local live evidence covers Assigned; the [billet author's run report](https://billet.readthedocs.io/en/stable/reference/upstream-references.html)
additionally describes Completed retaining zero on 2026-08-19. The Assigned shape
is also reported by a third-party user in [actions/scaleset#107](https://github.com/actions/scaleset/issues/107),
not confirmed there by a maintainer. Started zero is supported as an unknown-request
contract; its occurrence was not separately demonstrated by this investigation.
The fix must retain these unknown facts and real job/runner evidence with the valid
demand snapshot, then ACK the committed message, without acquiring request zero
or conflating unrelated Jobs/episodes.
The implementation now keeps zero-request facts in the UUID-keyed workflow
observation ledger while excluding them from request-based indexes, promotion
and conflict checks. Exact scoped Runner evidence and source episode times can
still establish execution; an unknown request alone cannot. No migration is needed.
Two real HTTP/listener/SQLite composition tests reproduced the production failure
before the fix and now pass, including Assigned/Started/Completed delivery, demand,
ACK, replay and no zero-request acquisition. Store/core regressions cover multiple
Jobs and anonymous facts without conflation and both known Runner completions.
All 716 local workspace tests pass (2 platform skips), with strict Clippy and
rustfmt. Linux packaging passed 709 workspace tests (2 skips) and both real
Terraform HTTP-backend integration tests.

Source `7c09b48` was deployed through PowerArmor `5933a55`. At 20:34 UTC, the
running package matched the build and both Fleets remained Ready. Session epoch 7
durably retained and ACKed messages `100000001` and `100000002`: the original
Assigned observation and its subsequent Completed observation both carried zero.
No request-zero acquisition was created. The Completed result was `canceled`, so
the Jobs view correctly reports assignment withdrawal, not workflow success.
All four configuration/revision table hashes and schema 16 were preserved.

This verifies recovery of the actual blocked message, not a successful Runner job.
Two separate follow-on problems remain: SQLite reported another transient busy
error at 20:30:59 UTC, and a JIT-created Generation reached Quarantined after its
Terraform plan failed. Read-only validation isolated a missing Linux AMD64 provider
content hash in the Proxmox template lock: the installed provider matches the
upstream 0.4.0 ZIP and its approved `zh`, but the lock lacks the platform's `h1`.
The fix requires a newly validated/published Template Revision and explicit Fleet
adoption; modifying the quarantined workspace's frozen lock is not recovery.
Neither that template repair nor the intermittent SQLite contention is fixed by
the unknown-request change.

## Scale Set labels protocol correction (2026-09-09)

Live GitHub.com validation isolated two errors in the mutable-labels implementation:
`Customer` label types returned HTTP 400 (`ArgumentNullException` for
`runnerScaleSet`), while valid `System` labels sent with PATCH returned HTTP 200
without changing the stored labels. A PUT containing only `labels` replaced the
set successfully. Both restoring the original label and applying the desired
labels were read back on the same Scale Set ID, with name, runner group and runner
settings unchanged and no registered, acquired, assigned or running work.

Fleet wiring now emits `System`, the wire enum recognizes GitHub's `System` and
`User`, and the narrow update adapter uses PUT. Failed updates log the operation,
Scale Set ID and actual HTTP status without response bodies or credentials.
Ownership, fencing, inventory and mandatory readback protections remain unchanged;
no database migration, Fleet revision or infrastructure replacement is required.
Spec 0001, spec 0002 and ARD-0027 record the corrected contract.

Composition regressions first reproduced `AccessVerificationFailed` for the
invalid type and `ScaleSetLabelsPending` for PATCH's unchanged response, then
passed with PUT/System. All 708 local workspace tests pass with 2 platform skips.
The Linux Nix package also passed 701 workspace tests (2 skips), strict Clippy,
and both real Terraform HTTP-backend integration tests.

Production source `35cf06f` was deployed through PowerArmor `232f1b0` on
2026-09-09. The running binary was verified against the built Nix package;
`pve-builder-tyo` recovered to Ready at desired/observed r2, its Replace Change
succeeded, and listener epoch 5 used the original owned Scale Set 9. Independent
GitHub reads confirmed `self-hosted` and `wanix-runners` and accepted the mixed
organization inventory. The Docker Fleet remained Ready. Schema 16, Fleet and
Template revisions, credentials and account bindings were preserved. This proves
access/labels recovery at that observation point, not sustained listener health or
a newly executed workflow job; the later Assigned-message failure is recorded
above. A separate intermittent
SQLite busy error had occurred before deployment; its historical lock holder was
not established by this protocol fix.

## Mixed organization Runner inventory (2026-09-09)

The production `pve-builder-tyo` failure was reproduced through its observed
`indexyz-org/r3` GitHub App binding. Installation, registration, runner-group and
Scale Set reads succeeded; the organization-wide inventory returned HTTP 200 with
384 ordinary Blacksmith runners lacking `runnerScaleSetId`. The adapter defaulted
that field to zero and rejected the whole list before filtering for Scale Set 9,
surfacing `AccessVerificationFailed` despite successful authentication.

The inventory validator now accepts zero/omitted membership while retaining count,
Runner ID/name and negative-membership checks. Fleet inventory contains only explicit
matches for the requested positive Scale Set ID. Exact-name lookup rejects missing
membership or a different returned name instead of manufacturing ownership or absence.
Unknown runners explicitly belonging to the current Scale Set still block adoption;
removal and Busy-safe cleanup semantics are unchanged. Spec 0001 §7 and ARD-0027
record the distinction between Target-wide inventory and Fleet ownership proof.

Before the fix, mixed-list wire tests and both production wiring regressions failed
with `AccessVerificationFailed`. After the fix, all 68 adapter tests and 16 listener
composition tests pass, including ordinary/foreign membership, malformed envelopes,
exact-name uncertainty, current-set unknown runners and subsequent labels updates.
Independent review found no remaining actionable issue in inventory/absence handling.
No schema migration, Profile/Fleet revision or infrastructure-template change is needed.
Full local workspace verification passes: 707 tests with 2 platform skips, strict
all-target/all-feature Clippy, rustfmt, diff checks and the 400-line Rust module limit.

## Mutable Fleet labels (2026-09-09)

[Spec 0002 §6.1](specs/0002-fleet-http-control-plane.md#61-mutable-scale-set-labels)
and [ARD-0027](ard/0027-reconcile-labels-on-owned-scale-sets.md) now allow labels
updates through the existing Fleet PUT/Revision/Change pipeline. The reproduced
HTTP/SQLite regression initially returned `409 immutable identity change rejected`;
it now accepts the change with a Busy Generation retained, stable identity/pins,
durable replay and stale-version rejection.

The supervisor updates only a proven-owned Scale Set. Migration 16 backfills the
new owned-ID marker only from positive Adopted IDs, preserving the distinction
between ownership and a conflicting candidate. Complete label-set comparison
supports removal and the empty-list System fallback. The shared Fleet effect gate
covers ownership reads/writes, PATCH and readback, including recovery. A second
current-authority check after inventory rejects an intervening revision, deletion
or Auth Context change. No labels update changes Runner Generations or invokes IaC.

Local verification: 697 Rust tests passed with 2 platform skips; all 168 browser
component tests passed. This includes 14 production listener composition tests,
5 label PATCH wire tests, HTTP admission and SQLite migration/ownership tests.
The composition tests prove same-ID add/remove/clear, ignored-success pending state,
unknown/candidate rejection, effect-gate serialization and stale-authority refusal.
An applied PATCH with a damaged response remains AccessBlocked with its owned marker;
recreating wiring reads back the update and restores Ready without a second PATCH.
Strict all-target/all-feature Clippy, rustfmt, Web build/lint, changed-Web formatting
and diff checks pass. Whole-Web formatting still reports the 10 untouched files
listed by the previous increment; no unrelated formatting changes are included.
The literal AGENTS `--workspace test` gate also passed: 556 tests, with 143
filtered/platform skips; the unfiltered workspace result above is the full gate.

Independent runtime/ownership review found a stale readback write outside the
effect gate; ownership reconciliation now holds the gate across that path too,
and the reviewed final implementation has no remaining actionable findings.
Production deployment and actual GitHub job routing are separate from these local
checks; no live Fleet labels are changed merely to exercise the feature.

## Authentication management pages and focused simplification (2026-09-09)

[Spec 0011 §6](specs/0011-multi-account-github-authentication.md#6-http-and-ui-contract)
and the [ARD-0015 amendment](ard/0015-route-one-github-app-profile-to-multiple-accounts.md)
define on-demand Re-auth, explicit policy publication using the reviewed Active
credential, and standalone create/policy/rotation pages. The local implementation
keeps the overview and selected connection at `/auth?key=...`.

The focused `simplify-codebase` change covers the Auth UI → protected HTTP routes →
Profile Registry → SQLite Candidate writer, plus the separate read-only GitHub App
adapter. Public full PUT, immutable revisions, the validation worker, activation and
runtime Handoff remain supported consumers. Real GitHub installation and production
deployment are outside this local verification boundary.

| Finding / disposition | Proof and consequence |
| --- | --- |
| S1 — remove modal lifetime ownership | `web/src/pages/auth.tsx` was the only production owner of create/rotate modal flags; `auth-form.tsx` owned an additional resource snapshot and inferred rotation from policy differences. Standalone routes now own lifetime, one shared form has explicit create/policy/rotate modes, and policy inputs use Active metadata. The cut removes both modal flags, the extra snapshot and inferred operation; it deliberately replaces long-form modals, preserves the overview, and leaves the short retirement dialog supported. Consumer tests now enter through routes and check fresh snapshots, draft retention, secret disposal and late responses. Confidence: high; risk is navigation/draft behavior, reversible in source. Topology: these direct React consumers, no dynamic registry. |
| S2 — keep one Candidate writer | `commits.rs` previously owned the complete Auth revision/change/audit/outbox/idempotency transaction. Both full PUT and policy updates now call `commits_auth.rs::commit_auth_candidate`; `commits_auth_policy.rs` adds only replay and exact Active credential-source checks before that same writer. This prevents two publication protocols from drifting. Existing PUT identity and persistence remain unchanged; the new policy operation has its own request identity and audit action. Atomicity, credential-copy, stale-base and accepted-replay tests are the decisive checks. Confidence: high; risk is transactional publication. No schema migration or dependency is added. Topology: the two admission paths converge on one SQLite writer. |
| S3 — retain trust and concurrency boundaries | The installation-link port remains separate from Profile Registry publication because it performs credential-bearing outbound GitHub reads. App identity/slug checks, redirect refusal, response sanitization, scopes/CSRF, desired-head CAS, Active-base checks and replay-before-credential-read have real external or concurrent consumers. Removing them would surrender authorization or retry guarantees; the candidate is rejected. Adapter and HTTP/SQLite race tests exercise those boundaries. Confidence: high; GitHub's live account chooser remains externally unverified. |

Operation receipt: scope is this Auth management increment, based on the existing
implementation at `7b3a0b9`; unrelated Proxmox work at `5f5cab2` is preserved.
No new pre-change full-suite run was taken for the simplification; prior results
below are historical, and the feature-specific and repository checks for this
increment are recorded separately. Changed artifacts are the linked contract and
glossary, Auth page/form/API consumers, Rust route/adapter/publication modules and
their tests. Net structural effect: two modal flags and one duplicate snapshot are
removed, one shared form and one Candidate transaction remain, and the requested
features add a route page and two endpoints. This is not a claim of reduced total
LOC. There is no new workflow framework, package, migration or persisted format.
Undo consists of reversing this increment's source diff, preserving other work;
no production configuration, database or GitHub installation needs restoration.

Verification passed for this increment:

| Layer | Command / evidence | Result |
| --- | --- | --- |
| Residue / structure | Search Auth page/form for modal flags, duplicate snapshot and Candidate policy seed; search both publication callers for `commit_auth_candidate`; `git diff --check` | Removed paths absent, one shared writer, clean diff; every Rust file is at most 400 physical lines |
| Narrow behavior | New installation-link adapter and real protected HTTP/SQLite tests; policy publication, concurrency, rollback, replay-after-cleanup and HTTP → v2 validator → activation tests | Passed; no PEM in policy payload, exact stored credential copied, old revision preserved, link reads leave durable state unchanged |
| Rust formatting / lint | `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets --all-features -- -D warnings` | Passed |
| Rust workspace | `cargo nextest run --manifest-path Cargo.toml --workspace --no-fail-fast --status-level fail --final-status-level fail` | 681 passed, 2 platform-specific tests skipped |
| Literal AGENTS filter | `cargo nextest run --manifest-path Cargo.toml --workspace test --status-level fail --final-status-level fail` | 546 passed, 137 filtered/platform tests skipped |
| Web build / lint / changed-file format | From `web`: `npm run build`, `npm run lint`, `oxfmt --check` on the 19 changed TypeScript files | Passed; Vite retains its bundle-size advisory |
| Component browser suite | From `web`: `npm test -- --workers=6` | 163 passed; fresh 390px create/policy/action renders inspected |
| Real HTTPS/OIDC browser suite | From `web`: `npm run test:oidc` | 8 passed, including the three new protected deep links and draft-preserving renewal |
| Independent reviews | UI against §6.1–6.3 and backend publication against §6.2, reviewed by agents who did not implement those surfaces | No actionable findings |

The first broad browser run found two obsolete modal test selectors; those now
assert standalone navigation and the final suite passes. Running the filtered Rust
suite while the OIDC fixture held its Windows executable caused a linker lock;
rerunning after the fixture stopped passed. A whole-Web formatting probe reports
10 unchanged files with existing style differences; this increment's 19 files pass
the scoped formatter. No production deployment or real GitHub installation has
been performed by this change. Local mocks verify GitHub protocol boundaries,
not account installation acceptance or live Fleet behavior after authorization.

## Proxmox runner template (2026-09-09)

[Spec 0022](specs/0022-proxmox-runner-template.md) and
[ARD-0026](ard/0026-provision-proxmox-runners-with-nocloud.md) are implemented locally.
The bundled `proxmox` source uses locked `indexyz/proxmox` 0.4.0 to manage a full
VM clone and a Generation-owned NoCloud ISO, with fixed DHCP and publisher-only
platform bindings. An explicit operator-managed VM image contract preserves
existing container content-pin rules. The image itself is not content-pinned.
Default import and Nix packaging retain root `.tftpl` assets; the database recovery
test restores all render sources after both local cache and archive are removed.

Local verification passed: 652 unfiltered Rust workspace tests (2 platform tests
skipped), the literal AGENTS filtered suite (532 passed, 122 skipped), strict
all-feature/all-target workspace Clippy, rustfmt, and the 400-line Rust file gate.
Actual Terraform 1.9.8 plus the locked provider against a loopback HTTPS PVE fixture
passed six negative plans, custom-script/JIT rendering, token splitting, upload →
clone → attach → start → stop → VM delete → ISO delete, and final empty state.
Normal refresh-enabled Destroy succeeds after the source template disappears.
The captured Create/Destroy plan projections also pass Shaula's production plan
admission API. Nine mocked guest shell tests cover handoff, failure propagation,
uppercase `CIDATA`, seed restrictions and prevention of a repeated registration.

Test entry points are `templates/proxmox/tests/conformance.py`,
`scripts/proxmox-conformance/test_guest_bootstrap.py`,
`crates/shaula-template/src/proxmox_tests.rs`, and the artifact library sync tests.
Real PVE guest boot, Linux permissions/PAM/systemd/cloud-init, DHCP, GitHub JIT/job
execution and safe removal remain unverified. Nix installation rules were inspected;
the Linux Nix package was not built and this feature has not been deployed.
These local checks do not constitute platform conformance or production readiness.

## Template approval controls and optional inputs (2026-09-09)

[Spec 0015](specs/0015-template-library-and-variable-discovery.md) and
[ARD-0019](ard/0019-store-template-sources-and-discover-terraform-variables.md)
now define direct checkbox editing for declared Fleet input approvals. Publication
and explicit Update policy replacement share the controls. Editing a value preserves
other approvals and their raw JSON tokens; manually added values remain visible and
removable. Invalid JSON disables the affected visual editor without rewriting its draft.

Optional scalar bindings remain editable and display default placeholders with a
per-field default action. Non-enumerated scalar parameters accept explicit custom
approvals; booleans use Yes/No radio controls. Defaults never become bindings or
approved values merely by rendering the form. Null defaults remain distinct from
omission, and empty strings, false, zero and large integers retain their types.

Local verification: nine focused browser regressions cover exact publication/Update
payloads, keyboard selection, custom scalar entry, defaults, preserved manual values,
invalid policy, and 390px layout. The full 125-case browser suite passed, and the
390px checkbox/input render was inspected. All 643 unfiltered Rust workspace tests passed
(2 platform-specific tests skipped), as did the literal AGENTS filtered command
(523 passed, 122 skipped), strict all-target Clippy, rustfmt and frontend build/lint.
Release delivery uses PowerArmor's locked `shaula` input; these UI tests do not
provision runners or submit a real GitHub job.

## Default template synchronization and Update (2026-09-09)

[Spec 0021](specs/0021-default-template-updates.md) and
[ARD-0025](ard/0025-sync-default-templates-and-explicitly-update-published-revisions.md)
have their original catalog synchronization and explicit Update flow implemented.
Startup validates the complete configured source set before replacing
the catalog in one transaction; obsolete entries disappear and old archives/revisions
remain immutable. Missing configured roots, duplicate keys and invalid templates fail
without publishing a partial catalog. The NixOS module uses the current package's
stable default directories; an empty list clears only the default catalog.

Published templates expose **Update** and an explicit **Update from default** review.
The server inherits protected bindings and omitted policy from the exact If-Match base;
explicit policy replacement preserves JSON numeric precision. Conditional publication,
validation and activation share the existing path, including authoritative replay after
concurrent idempotency races. Fleet and Generation pins are not changed.

The persisted-source amendment is implemented locally (2026-09-09). Migration m0015
adds nullable per-Revision `source_key` without a catalog foreign key and recovers only
unique digest/engine/known-platform associations from the old catalog. PUT/Update
validate the reviewed source and persist it in the revision transaction; revision reads
return `sourceKey`. Source identity participates in NoOp and idempotency, while accepted
replays and absent-source legacy request identities survive catalog changes. Historical
configuration and Fleet/Generation pins remain unchanged.

Update uses the saved key without asking again; missing or incompatible saved sources
remain unavailable instead of selecting a replacement. Legacy drafts may preselect an
exact match or sole platform candidate, but only explicit publication establishes the
association. Default publication fixes the source engine; archive/existing publication
does not inherit a source. Source-only and policy-only updates are supported, and a
200 NoOp returns to the template with a no-change result rather than polling an empty
Change ID. The authenticated Update deep link supports direct navigation and refresh.

Amendment verification passed: 643 unfiltered workspace tests (2 platform-specific
tests skipped), the final literal AGENTS filtered command (523 passed, 122 skipped),
strict all-target Clippy, rustfmt, frontend build/lint and formatting of 14 changed Web
files, and the complete 116-case browser suite. Independent Store/API and frontend
reviews passed after fixing NoOp handling. Migration tests exercise atomic rollback,
retry, SQLite backup/restore and retained historical pins. All 5 real HTTPS/OIDC browser
tests passed: the new flow publishes from Docker, reads the stored source, clears and
explicitly restores the association across three real daemon-activated revisions,
reloads Update from database state, then verifies a real unchanged-policy NoOp. Desktop
and mobile renders were inspected. The fixture isolates bundled modules from ignored
retired directories in an existing checkout. Release delivery uses PowerArmor's locked
`shaula` input; these tests do not create a real GitHub job or provision Docker/Kubernetes
resources.

The verification below describes the original synchronization and Update implementation.

Local verification: 632 workspace Nextest tests passed (2 platform-specific tests skipped),
strict all-target Clippy and rustfmt passed; the 95-case browser suite and one additional
mobile action-layout regression passed (14 Update cases), with desktop/mobile screenshots
checked. Targeted SQLite/HTTP tests cover
secret inheritance/redaction, history/Fleet preservation, permission and input rejection,
exact-base retries, concurrent CAS and concurrent idempotency. A clock barrier forces both
requests past the initial replay lookup: disabling result reclassification makes all three
race tests fail, and restoring the identical production file makes them pass. Web build/lint and changed
Web files' format checks passed; the full Web format check still reports 14 unchanged
files with pre-existing formatting differences. These checks do not establish a real
GitHub job run or Kubernetes resource conformance.

## Official container images and host bootstrap (2026-09-09)

[Spec 0020](specs/0020-official-container-runner-bootstrap.md) and
[ARD-0024](ard/0024-bootstrap-official-runner-images-outside-containers.md) replace
the custom-image delivery design for new Docker/Kubernetes Template revisions.
The bundled sources now pin official Runner 2.337.0 and declare
`shaula.container-bootstrap/v1` with v1 inputs. Docker is created stopped;
Kubernetes uses Pending plus a required missing `.setup_info` Secret item.
Runner commands invoke the official Listener directly with native JIT input;
custom Dockerfile/shim/helper and the v2 source generator have been removed.

`shaula-template/src/runtime_bootstrap*.rs` implements the fixed host delivery
and startup path; `shaula-store/src/lifecycle_bootstrap*.rs` adds the atomic
`BootstrapStarting` admission transition so an uncertain bootstrap is not
replayed. Before ApplyStarting, saved-plan admission verifies the official image,
native Listener/JIT input and stopped-container or missing-Secret-key startup
gate; critical unknown fields and provisioner hooks are rejected. Actual resource
identity and the gate are checked again before host bootstrap.

Local Rust validation passed 609 workspace tests (2 skipped), rustfmt and strict
Clippy across all targets. The full test run explicitly enabled the captured
Docker 3.0.2 and Kubernetes 2.33.0 provider-plan oracles. The Kubernetes plan was
produced against an isolated mock Namespace endpoint, not a live cluster.
Terraform 1.9.8 fmt, readonly locked init and validate passed for both templates
in isolated Linux copies. The Kubernetes 2.33.0 provider lock now contains real
HashiCorp checksums rather than a comment-only placeholder.

Current production composition remains daemon-owned local-state Runtime; independent `shaula job`/HTTP-state
integration is not established by this feature. Existing retained v1/v2 artifact,
inputs, pins and state are not rewritten; the old HTTPS listener remains a
compatibility path for already retained v2 Generations; new publication/Create
requires the official-image host bootstrap contract. Existing Fleets pinned to
legacy container Templates must select a newly published official revision
before further Runner creation; no automatic mutation of their pins occurs.

The external Docker harness now prepares the stopped container, copies and
reads back a fixed smoke marker from the host, records possible start before
starting, and inspects the official Listener. Its marker is not the production
apply projection. Local Python validation passed 35 tests, with 5 Linux-only
checks skipped on Windows. This is neither production archive integration
acceptance nor full conformance. The historical 2026-09-08 custom-image smoke
does not validate the new official-image/host-bootstrap tuple. The isolated
[2026-09-09 official Docker probe](evidence/docker-official-bootstrap-2026-09-09/README.md)
verified stopped Create, host file copy/readback, UID 1001 readability, official
Listener rejection of synthetic invalid JIT, and delete-only Destroy/empty state.
It did not run a real GitHub job or change production service/Fleet configuration.
No new Docker or Kubernetes first-job Set up job acceptance is claimed here.

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
- The original implementation included a container helper and v2 source
  generator. Those source paths have since been removed by the official-image
  increment above; retained v2 artifacts keep their original bytes and protocol.

Local validation on 2026-09-09: the full workspace nextest gate passed 579 tests
with 2 existing external-environment tests skipped; workspace Clippy and rustfmt
passed. The browser suite passed 82 tests, and Linux bootstrap/helper and v2 source
generation passed 21 and 2 tests respectively. Coverage includes Jobs
identity/reassignment and migration replay, archive retention/recovery/capture,
HTTP permissions, embedded routes and browser behavior. These checks are not real
Runner acceptance.

The current production composition still uses the daemon-owned local-state
Runtime; future Lifecycle Worker/HTTP-state integration remains staged. The
original shim/v2 delivery design has been superseded for new bundled Templates
by spec 0020 above. The 579-test and Python helper results are historical evidence
for that earlier increment, not validation of the new official-image bootstrap.
Older generations have no retroactively generated logs or metadata.

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
  sessions and persist-before-ACK message ingestion/acquisition. WaitingOnline
  generations are driven by the spec 0024 readiness reconciliation
  (inventory-online → Idle; readiness timeout → CleanupRequired, covering
  ephemeral JIT runners that self-deregister after their single job).
  Fleet decommission converges end-to-end in wiring tests: the deletion-
  marked fleet's cleanup supervisor binds the last admitted spec revision
  (DELETE writes no spec row), retires all owned generations and lands
  tombstone + `Decommissioned` + Change `Succeeded` in one transaction.
  Complete busy-state classification, operation recovery and real-platform
  decommission acceptance remain open. Store and scripted-listener
  tests alone do not prove these complete external workflows.
- End-to-end real-GitHub validation and the Go-oracle differential suite
  (`references/scaleset`, pinned commit) have not been executed.
- Bundled-profile conformance remains a runtime evidence gap, independent of
  automatic static activation under spec 0017. The Docker provider
  now has a real Terraform-generated lock for `kreuzwerker/docker` 3.0.2 and
  a historical privately imported shim image pin. Current official image pins
  and host bootstrap supersede that source; Kubernetes full runtime acceptance
  remains staged. `scripts/docker-conformance/` supplies
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
| Zero-Occupancy barrier for changed `template_inputs` | Implemented (2026-09-22): `commits_fleet.rs` compares semantic inputs and rechecks Generation/open-operation occupancy under the commit writer lock. `tests/fleet_inputs_fence.rs` covers Create-after-admission, an open effect after Destroyed, and unchanged inputs during capacity-only updates. |
| Lower max below current Busy/Occupancy | Normative behavior is stop new admission and retire safely, not reject solely for current Occupancy or kill Busy. Existing arithmetic does not prove complete listener/worker scale-down acceptance. |
| Reachable cross-Auth handoff with an idle session | Spec 0002 removes an idle session as an admission blocker while retaining the zero-Occupancy/effect barrier. The listener repair adds exact-context session gates and quiescence; real multi-account idle-session/race acceptance remains a release gate. |
| Profile retirement self-head release | Implemented (2026-09-22): level-triggered `retirement_scan.rs` rechecks live Fleet/pool pins, Generations/effects, sessions/acquisitions and worker/validator claims under one writer transaction, clears active/observed heads and completes Retire Changes. Desired revision remains a historical high-water mark; protected bytes/history are not GC'd. New pool references recheck the retirement fence at commit. HTTP replay, unused Active heads, consumer release, retained pool revisions, expired claims, transaction rollback and restart are covered by `profile_retirement*` tests. Local validation: strict workspace Clippy and 440 Store/binary tests passed (2 skipped); this is not remote cleanup acceptance. |
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
  repacking and restores missing execution material. The original one-time source
  seed is superseded by [spec 0021](specs/0021-default-template-updates.md): startup
  atomically synchronizes the complete configured default catalog, retaining old
  archives and published revisions. Source changes do not publish/activate a Profile.
  Runtime reads and Create/Destroy also verify cache
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

## Forgejo runner backend: in-progress Pool implementation (2026-09-11)

[Spec 0026](specs/0026-forgejo-runner-backend.md) and [ARD-0033](ard/0033-admit-forgejo-through-a-pool-backend-first.md) remain draft/proposed. This is **not yet an A1–A8-complete runner backend**.

Implemented and covered by local adapter/SQLite tests:

- Provider-specific HTTP routes, bounded registration bodies/response/pagination handling, `null` jobs and undeclared labels, label-name filtering, and protected one-shot registration material. Missing/malformed or unexpected successful/server-error registration responses are uncertain, never authorization to repeat POST. Server releases below 15 and runner image version tags below 13 (or unverifiable/prerelease versions) are rejected; exact image-content conformance still belongs to R4.
- A separate Pool driver with durable registration intents, Forgejo identity table, captured Fleet fences, restart classification, hard occupancy limits, inventory readiness/Busy transitions, and independent registration/resource cleanup. Exact-ID reads confirm absence; mutable paginated inventory alone cannot authorize destruction. Failed polls do not refresh the old demand timestamp.
- Independent `forgejo_token` publication/probing and non-secret API read views, with separate active/candidate validation metadata. Rotation appends a Fleet revision rather than entering GitHub Auth Handoff. User-scoped tokens retain their verified principal ID; organization/repository scopes retain their numeric target ID across rotation and execution. Deleting Fleets retain their credential dependency until cleanup finishes.
- Non-secret identity in Terraform input and provider-aware variable discovery; token delivery outside tfvars through the Docker/Kubernetes bootstrap boundary. The initial container contract admits explicit `:host` labels only. Docker-in-Docker and VM bootstrap are rejected, not silently guessed.

Still open:

- **Safe idle expiry/drain.** The runtime evidence hook defaults to `Unknown`; remote `idle` does not authorize DELETE. Occupied idle generations can consequently block scale-down, rotation and decommission. The pinned runner's [single-task poller](https://code.forgejo.org/forgejo/runner/src/commit/667c8d975b9255e7bf32164e012146f68f0022c6/internal/app/poll/single.go) shares cancellation with task execution; blindly signalling a runner is not a proven busy-safe drain. The full-contract release gate remains unclosed. The non-waiting experiment below did not prove safe cleanup. The separately authorized [shared hard lifetime](runner-lifetime.md) bounds normal idle/Busy retention by permitting task interruption at expiry; it is not busy-safe drain and leaves ownership failures quarantined. [Source evidence, the proposed acquisition-fence prerequisite and its acceptance matrix](forgejo-drain.md) record the release blocker; no upstream fence/drain implementation is present.
- Uncertain registrations that have not declared labels are quarantined, because the spec's joint-label ownership proof cannot identify them yet. This retains occupancy rather than silently leaking/replacing them.
- R4 image-content/platform conformance, Forgejo UI/jobs projection, exact scope/permission evidence, and A1–A8 real-platform evidence. The existing bundled Docker/Kubernetes sources now have a Forgejo publication option (2026-09-21 increment above), rather than new template kinds. The pinned [official image](https://code.forgejo.org/forgejo/runner/src/commit/667c8d975b9255e7bf32164e012146f68f0022c6/Dockerfile) uses `/data`, UID 1000 and `dumb-init`; bootstrap validators reflect those facts, but have not been validated against a live container in this increment.

Historical local checks (2026-09-12), superseded by the 2026-09-21 workspace checks above: `cargo fmt --all -- --check`, targeted strict Clippy for the changed Rust packages, and the full unfiltered `cargo nextest run --manifest-path Cargo.toml --workspace --no-fail-fast --status-level fail --final-status-level fail` pass (803 tests passed; 2 skipped). The workspace-wide Clippy invocation remains blocked by the Windows Vite native dependency (`UNLOADABLE_DEPENDENCY`) and `spawn EPERM` while building `shaula-http`; this is an environment blocker, not a Rust diagnostic. These checks are not real-platform acceptance evidence.

The existing `.github/workflows/forgejo-e2e.yml` exercises Forgejo 16.0.4 with runner 13.1.0 using shell API calls. Run [34694263009](https://github.com/5aaee9/shaula/actions/runs/34694263009) passed registration, a matching job, and ephemeral disappearance, but it is not an end-to-end test of the new Rust Pool driver and does not provide the full A1–A8 evidence. The per-item local/real evidence split is recorded in [the A1–A8 matrix](evidence/forgejo-a1-a8-2026-09-12/README.md).

**2026-09-21 non-waiting protocol experiment:** [six real official-image cases](evidence/forgejo-one-job-2026-09-21/README.md) verify nominal success/failure cleanup and reproduce the counterexample: a lost assignment response produces runner exit 2 and inventory `idle` while the job is already `running`. The `--wait` control recovers and completes. The reusable harness removes its disposable containers/volumes; it does not exercise the Rust Pool/Terraform path. Production retains `--wait`; no production proxy was added. The later shared hard-lifetime policy above is an explicit Busy-safe exception, not a consequence of exit status or idle inventory.

## Known accepted limitations (per ADR)

- Same-Runner-Execution-Domain JIT process-inspection risk (ADR-0004).
- Name-based Kubernetes deletion residual risk (ADR-0006).
- Single ambient host-admin trust domain for IaC children (ADR-0008).
