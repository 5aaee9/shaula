# Implementation Status (Phase Boundary)

Status: Draft implementation of the Shaula v1 specification set.
Date: 2026-09-06

This file records, honestly and per review round, which specification
capabilities are implemented and which remain staged for later phases of
the delivery plan (spec 0001 §17).

## Implemented and tested

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
  idempotent replay and conflict, trusted actor context (401/403),
  sanitized problem responses, loopback-only bind fail-closed.
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
  material remain available. Retained Profile heads keep retirement visibly
  `Blocked(ResourceInUse)` pending retention/reference clearance.
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

## Staged (next phases; not yet wired or externally validated)

- The per-Fleet capacity/ownership/cleanup supervisor and Auth validator are
  started by the binary. The production session listener, persist-before-ACK
  message ingestion/acquisition, online/busy inventory classification, operation
  recovery and complete Fleet decommission/tombstone workflow are still missing.
  Existing persistence primitives alone do not implement these workflows.
- End-to-end real-GitHub validation and the Go-oracle differential suite
  (`references/scaleset`, pinned commit) have not been executed.
- Bundled-profile conformance harness (real Kubernetes/Docker runs) and
  provider lock checksums — attestation remains the activation gate. The
  bundled templates REQUIRE a runner image that carries the reviewed
  bootstrap-shim at `/usr/local/bin/bootstrap-shim`; that shim-bearing
  image is built and pinned by the conformance harness, not by the
  template bundle. Digest-pinned `runner_image_digests` in the manifests
  are finalized by the same harness.
- OTLP export pipeline and OTel SDK instrumentation (bounded in-process
  counters and their call sites exist; no span/export pipeline yet);
  remaining
  endpoint surface (list pagination cursors, revision reads, artifact
  metadata GET), and Profile reference clearance/retention/GC.

## Known accepted limitations (per ADR)

- Same-Runner-Execution-Domain JIT process-inspection risk (ADR-0004).
- Name-based Kubernetes deletion residual risk (ADR-0006).
- Single ambient host-admin trust domain for IaC children (ADR-0008).
