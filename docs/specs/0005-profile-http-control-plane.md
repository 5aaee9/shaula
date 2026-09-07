# Shaula v1 Profile HTTP Control-Plane Specification

- Status: Draft
- Date: 2026-09-04
- Managed resources: Template Profiles and GitHub Auth Profiles
- Persistence: SQLite plus content-addressed Template artifacts
- Secret-at-rest decision: PAT, GitHub App private key, and schema-sensitive Template bindings may be plaintext in SQLite

This specification extends the [Fleet HTTP Control-Plane Specification](0002-fleet-http-control-plane.md). Related decisions are [ADR-0005](../ard/0005-manage-fleet-desired-state-through-http-and-sqlite.md), [ADR-0007](../ard/0007-use-target-bound-github-auth-profiles.md), [ADR-0009](../ard/0009-manage-profile-resources-through-http-and-sqlite.md), and [ADR-0013](../ard/0013-require-openid-connect-for-all-http-access.md). Inbound authentication is normative in [spec 0009](0009-mandatory-openid-connect.md); GitHub Auth Profiles remain outbound credentials, not OIDC identities.

## 1. Outcome

One `shaula serve` HTTP Interface manages three desired-resource families：

- Fleet；
- Template Profile；
- GitHub Auth Profile.

`template_profiles` and `github_auth_catalog` are not daemon-bootstrap truth sources. Their desired heads, immutable Revisions, conformance attestations, status, Changes, idempotency and audit facts live in SQLite. Template bytes are uploaded through HTTP and stored in the protected content-addressed artifact store. GitHub PAT/App private-key bytes and schema-sensitive Kubernetes/Docker binding values are accepted as write-only HTTP fields and stored in their immutable SQLite Revision rows.

The bootstrap file remains limited to native daemon concerns：storage paths, database, HTTP security/listen policy, IaC engine executables and installation policy, concurrency/timeout limits, artifact limits, and OpenTelemetry/logging.

## 2. Module shape

```mermaid
flowchart LR
    Client["Authorized client"] --> HTTP["HTTP Adapter"]
    HTTP --> Registry["Profile Registry Module"]
    Registry --> DB["SQLite revisions / changes / audit"]
    Registry --> Artifact["Template Artifact Module"]
    DB --> Validator["Async Profile validators"]
    Artifact --> Validator
    Validator --> IaC["Terraform static validation"]
    Validator --> GitHub["GitHub read-only validation"]
    Registry --> Fleet["Fleet Registry ref resolution"]
```

The Profile Registry is a deep Module. Its Interface accepts typed Template/Auth mutations, conformance attestations and queries, resolves Fleet references, and returns typed results. It hides SQLite layout, revision promotion, attestation validation, references, outbox, idempotency and audit.

Template Artifact storage is a separate Module because large content-addressed bytes, extraction safety, atomic publication and garbage collection differ from SQLite resource mutation. GitHub credential bytes leave the Registry only through an internal scoped handoff to the GitHub Access Module；they never cross Fleet or Template Runtime Interfaces, never enter a Terraform input and never enter a Runner. Sensitive Template bindings leave the Registry only through an exact-Revision handoff to the approved IaC child；they never enter a Runner or workflow.

Template Profile and GitHub Auth Profile remain distinct typed resources. A generic untyped catalog/EAV body is forbidden.

## 3. Common resource semantics

Each logical Profile has a stable key/incarnation, monotonic desired revision, active revision, observed revision, immutable revision history, mutable status, mutation fence and tombstone.

Template and Auth activation intentionally differ：

```text
Template: Pending -> Validating -> Ready -> Active -> Retiring -> Retired
Auth:     Pending -> Validating --------> Active -> Retiring -> Retired
```

- `desired_revision` is the latest conditionally accepted mutation.
- `observed_revision` is the latest asynchronously classified revision.
- `Ready` is Template-only and means built-in static validation passed；it is not selectable.
- A Template Candidate needs an accepted exact conformance attestation for `Ready -> Active`；an Auth Candidate instead needs successful identity/access validation for `Validating -> Active` and never consumes a Template attestation.
- The current `active_revision` is the only Revision that may receive a new Fleet reference, including an explicit exact-revision reference.
- A rejected or superseded Candidate never replaces the previous active Revision.
- An already-admitted Fleet's exact Template reference remains usable for normal reconciliation, future Create, Destroy and recovery during controlled retirement until the Fleet releases it.

Every Template/Auth Profile-spec PUT uses a strong ETag, `If-None-Match`/`If-Match`, a bounded `Idempotency-Key`, an authenticated actor and strict JSON. An effective Profile-spec mutation atomically commits an immutable Revision, Profile Change, sanitized audit fact and outbox marker before returning `202`. Attestation PUT and Profile DELETE use the same conditional/idempotent discipline but commit their endpoint-specific immutable record or retirement state plus Change/audit/outbox；they never create or modify a Template Profile Revision. No HTTP handler invokes Terraform or GitHub. An identical valid Profile-spec PUT returns replayable `200` without a new Revision or Change.

For a mutation containing a GitHub credential or sensitive Template binding, the idempotency record points at the committed Auth or Template Revision. A retry compares any supplied secret only in protected memory with that revision and returns its sanitized response；the record contains no reusable body or secret-derived verifier. Reusing the key with different content returns conflict. Profile DELETE is safe retirement, not row deletion. It immediately rejects new references and remains `Blocked(ResourceInUse)` while any Profile desired/active/observed head, Fleet-to-Template-Revision/attestation exact pin, Fleet desired/observed Auth tuple, in-flight effect/session, Decommission cleanup, non-terminal Generation/Operation or recovery record needs it. `Blocked` cannot release a reference. Only after every reference clears may retention/GC write a tombstone；there is no force delete.

## 4. HTTP Interface

All paths are below `/api/v1`：

| Method | Path | Meaning |
| --- | --- | --- |
| `PUT` | `/template-artifacts/sha256:{digest}` | Idempotently upload an immutable Profile archive |
| `GET` | `/template-artifacts/sha256:{digest}` | Authorized metadata only, not artifact download |
| `GET` | `/template-profiles` | Paginated Profile summaries |
| `PUT` | `/template-profiles/{profileKey}` | Create or publish a Candidate Revision |
| `GET` | `/template-profiles/{profileKey}` | Desired/active metadata and ETag |
| `GET` | `/template-profiles/{profileKey}/status` | Validation and reference status |
| `GET` | `/template-profiles/{profileKey}/revisions/{revision}` | One immutable sanitized Revision |
| `DELETE` | `/template-profiles/{profileKey}` | Retire after all references clear |
| `PUT` | `/template-profiles/{profileKey}/revisions/{revision}/attestations/{attestationKey}` | Submit one immutable conformance result |
| `GET` | `/template-profiles/{profileKey}/revisions/{revision}/attestations/{attestationKey}` | Read bounded attestation metadata |
| `GET` | `/github-auth-profiles` | Paginated redacted Auth summaries |
| `PUT` | `/github-auth-profiles/{profileKey}` | Create or rotate a Candidate Auth Revision |
| `GET` | `/github-auth-profiles/{profileKey}` | Redacted desired/active metadata and ETag |
| `GET` | `/github-auth-profiles/{profileKey}/status` | Validation, rollout and reference status |
| `GET` | `/github-auth-profiles/{profileKey}/revisions/{revision}` | One immutable redacted Revision |
| `DELETE` | `/github-auth-profiles/{profileKey}` | Retire after dependent Fleets clear |
| `GET` | `/profile-changes/{changeId}` | Query asynchronous validation/activation/retirement |

Artifact upload declares digest and exact compressed/content length before the body is accepted. Shaula streams to a temporary file under a configured limit, verifies digest and safe archive shape, durably renames to its final content-addressed path, and returns `201` or idempotent `200`. Path traversal, links/reparse entries, devices, oversized expansion and duplicate normalized paths are rejected. Upload does not create, attest or activate a Template Profile Revision.

## 5. Template Profile resource

A Template Profile PUT contains an exact uploaded artifact digest, engine reference, fixed Profile bindings and the allowed Fleet-input policy. It contains no Fleet or Runner desired state and MUST NOT contain `platform` or `bindings_contract` fields；strict JSON rejects either field as a second authority.

```json
{
  "artifact_digest": "sha256:...",
  "engine_ref": "terraform",
  "bindings": {
    "namespace": "actions-runners"
  },
  "fleet_input_policy": {
    "size_class": ["standard", "large"]
  }
}
```
The artifact's versioned binding schema marks which fields are sensitive. Those fields are write-only: the original bytes are retained in the immutable Template Profile Revision so restart, Create, Destroy and recovery can reproduce the exact binding, while every GET/list/status/revision/attestation response exposes only schema-approved non-secret fields and bounded presence metadata. No response exposes a secret value, prefix, suffix, hash or length. External secret references are not a v1 storage mode.
The wire field named `bindings_digest` is an opaque server-issued commitment to the exact immutable binding Revision. It MUST NOT be an unkeyed digest of sensitive plaintext or otherwise permit offline validation of a guessed secret. It may appear in protected envelopes and attestation subjects only under that non-verifier property；otherwise it is secret and must be redacted. Its exact construction and encoding remain an open contract detail.



The versioned `shaula-profile` manifest inside the artifact is the sole authority for normalized `platform` and `bindings_contract`. Shaula derives and exposes those values after parsing the artifact；the PUT body, binding names and attestation cannot override them. A Docker artifact therefore declares Docker and `shaula.bindings.docker/v1` in its manifest and may expose a trusted `docker_host` binding schema；the Profile PUT supplies only a value allowed by that schema.

A Profile incarnation fixes its manifest-derived `platform` and `bindings_contract`. Changing from Kubernetes to Docker, or to an incompatible bindings contract version, requires a new Profile key. Compatible artifact, binding or bounded-input-policy changes create Candidate Revisions under the same key.

Asynchronous static validation：

1. Re-open the already published artifact by digest.
2. Verify archive containment, manifest version, manifest-derived `platform`/`bindings_contract`, declared inputs/outputs, provider lock checksums and engine constraints.
3. Reject undeclared files, missing lock file, arbitrary executable hooks, a caller-supplied platform authority and Profile/Fleet input confusion according to policy.
4. Run isolated non-mutating Terraform initialization/validation with bounded output and no infrastructure credentials.
5. If this Candidate is still desired and all checks pass, transition it to `Ready`；static validation alone never activates it.

### 5.1 Conformance attestation and activation

A `Ready` Template Candidate is not selectable until a trusted external harness tests the exact runtime subject and an authorized caller submits a durable immutable attestation such as：

```json
{
  "result": "passed",
  "subject": {
    "artifact_digest": "sha256:...",
    "dependency_lock_digest": "sha256:...",
    "providers": [
      {"source": "hashicorp/kubernetes", "version": "x.y.z", "checksums_digest": "sha256:..."}
    ],
    "engine": {"kind": "terraform", "version": "x.y.z", "binary_digest": "sha256:..."},
    "runner_image_digests": ["ghcr.io/actions/actions-runner@sha256:..."],
    "bindings_digest": "opaque-binding-revision-commitment",
    "runtime_policy_digest": "sha256:runner-runtime-trust-policy",
    "platform": "kubernetes",
    "bindings_contract": "shaula.bindings.kubernetes/v1",
    "suite": {"name": "shaula-template-conformance", "version": "v1"}
  },
  "evidence_digest": "sha256:...",
  "completed_at": "timestamp"
}
```

The canonical attestation subject binds the artifact digest, dependency lock, exact provider source/version/checksum set, engine implementation/version/binary digest, finite runner-image digest set, protected opaque `bindings_digest`, required runtime/trust policy digest, manifest-derived `platform`/`bindings_contract` and conformance-suite version. The commitment is equality evidence for the immutable Revision, not a public hash of its sensitive values. The runtime/trust policy captures both enforced controls and explicitly accepted limitations；it does not imply workflow/Runner process isolation. The Registry recomputes the expected subject from the artifact, manifest, lock, exact engine binary, admitted binding revision and bounded image/runtime policy；a submitted field can only match that authority, never redefine it.

Attestation PUT requires `template.attest`, `If-None-Match: *`, bounded idempotency and an authenticated request-context actor. It atomically stores the attestation, canonical subject digest, Profile Change/outbox and an audit fact containing actor, Profile Revision, attestation key, subject digest, suite version and sanitized result；raw test output, bindings and credentials are excluded. An exact replay returns the original result；the same key with a different canonical body conflicts.

Only a `passed` attestation whose subject exactly matches the still-current desired `Ready` Candidate may gate an atomic `Ready -> Active` transition and freeze its `active_attestation_id`. A failed, mismatched or stale attestation remains durable and audited but cannot activate the Candidate or replace the previous active Revision.

A Fleet may submit a bare Profile key for convenience or an exact `{key, revision}`, but admission of a new or replacement reference accepts only the current `active_revision` and persists its exact artifact digest and active attestation in the immutable Fleet Revision. An already-admitted Fleet may keep using an older retained exact pin for normal reconciliation, future Create, Destroy and recovery until it releases that reference. Publishing or attesting a newer Revision never changes an existing Fleet/Runner；switching a Fleet follows the zero-Resource-Occupancy rule.

## 6. GitHub Auth Profile resource

The two request shapes are discriminated and strict：

```json
{
  "kind": "github_app",
  "app_id": "123456",
  "installation_id": "789012",
  "private_key": "-----BEGIN RSA PRIVATE KEY-----\n...",
  "target_allowlist": [
    {"kind": "organization", "owner": "example-org"},
    {"kind": "repository", "owner": "example-org", "repository": "example-repo"}
  ]
}
```

```json
{
  "kind": "pat",
  "token": "github_pat_...",
  "target_allowlist": [
    {"kind": "organization", "owner": "example-org"}
  ]
}
```

`private_key` and `token` are write-only secret fields. SQLite stores their original bytes in the immutable Auth Revision. They are omitted from every response, event, audit payload and diagnostic representation. A response exposes only `credential_present: true`, kind, non-secret identity, Target policy, Revision/state and timestamps；it exposes no prefix, suffix, hash or encrypted/plaintext representation.

The following identity fields are immutable for one Auth Profile incarnation：kind, App ID, installation ID, initial authenticated PAT principal and normalized Target allowlist. Changing any of them requires a new Profile key and explicit Fleet reassignment. A Fleet may replace its Auth Profile key only at zero Resource Occupancy with no active acquisition, GitHub or Runner operation；its Target and Scale Set identity remain immutable. A replacement PUT on the existing key is credential rotation only：new App private-key bytes for the same App/installation, or a new PAT that authenticates as the same GitHub principal. A PAT whose principal cannot be proved equal is a principal migration, not rotation.

Asynchronous validation and activation：

1. Parse the credential without logging parser content.
2. Construct only the matching Rust `shaula-scaleset` client；never try the other auth kind.
3. Perform bounded read-only identity/access checks for the declared Target policy and all currently dependent Fleet Targets.
4. For PAT rotation, require the authenticated principal identity to match the Profile's fixed principal.
5. If validation fails, mark the Candidate `Rejected` and leave the old active credential untouched.
6. If validation passes and the Candidate remains desired, atomically promote it and enqueue every dependent Fleet.
7. Same-Profile promotion sets each dependent Fleet's desired Auth Revision Ref to the full tuple `(profile_key, revision)`；every GitHub effect records the exact tuple it used.
8. Same-Profile promotion and an admitted zero-occupancy cross-Profile Fleet replacement enter the same durable Auth Handoff state machine.
9. Handoff quiesces new acquisition and waits for every acquisition crossing the fence to return or receive durable outcome classification.
10. For a bound Fleet, handoff uses the desired tuple only for authenticated read-only ownership proof of the persisted Scale Set ID and immutable identity.
11. For an unbound Fleet, handoff only validates access and records the desired access context；it does not create/adopt, bind a Scale Set ID, establish a session or mint JIT configuration.
12. Handoff durably records its classification and advances the observed Auth Revision Ref to the exact desired tuple. Equality means Auth handoff completed, not that a Scale Set or listener session exists.
13. Ordinary Fleet reconciliation exclusively owns create-or-adopt, Scale Set ID binding and session establishment/replacement；acquisition resumes only after that reconciliation establishes a ready session for the observed tuple.
14. A Decommissioning Fleet permanently rejects new acquisition but may run this state machine in cleanup-only mode；it never establishes an acquiring session, creates/adopts, changes the Scale Set ID or mints JIT configuration.
15. An old Auth Revision becomes GC-eligible only when no Profile desired/active/observed head, Fleet desired/observed tuple, in-flight effect/session, Decommission cleanup or recovery record references it. `Blocked` retains every such reference and cannot be used as acknowledgement or release.

Production validation, handoff and reconciliation use only the Rust `shaula-scaleset` implementation. A fixed, reviewed Go oracle pinned to an exact `github.com/actions/scaleset` commit exists solely for conformance/differential tests；it is neither shipped nor invoked by production Shaula.

Keeping the old active Revision until Candidate validation succeeds is staged activation, not request-time fallback. Once a Fleet switches, a new credential failure does not cause it to try an older Revision automatically.

A Fleet Spec may name a stable Auth Profile key for operator convenience, but its durable `desired_auth_ref` and `observed_auth_ref` are always complete `(profile_key, revision)` tuples, and every operation resolves one exact tuple. This controlled handoff is a control-plane reference change；it neither updates nor replaces an existing Runner Resource.

## 7. SQLite and artifacts

The conceptual schema adds typed tables：

- `template_profiles`, `template_profile_revisions`, `template_profile_status` and immutable `template_conformance_attestations`；
- `github_auth_profiles`, `github_auth_profile_revisions`, `github_auth_profile_status`；
- `profile_changes`, `profile_outbox`, `profile_idempotency_records`, `profile_audit_records`；
- `template_artifacts` and reference counts/retention metadata；
- explicit Fleet-to-Template-Revision-and-attestation relations plus `fleet_auth_handoffs` containing desired/observed Auth `(profile_key, revision)` tuples, fences, classifications and exact references held by in-flight effects, sessions, Decommission cleanup and recovery.

Auth Revision rows contain PAT/App private-key plaintext, and Template Revision rows contain plaintext values for schema-sensitive bindings. SQLite main/page files, WAL/SHM, online and migration copies, crash dumps and backups MUST receive credential-grade access, retention and disposal. Application-level encryption is not a v1 requirement；deployment-level full-disk/filesystem/backup encryption is strongly recommended.

The atomic backup/restore consistency set is SQLite, Template artifact store and all per-runner Workspaces/state. Restoring only one member is unsupported. Because GitHub credentials and sensitive Template bindings reside in SQLite, no separate external secret-store restore is required for either in v1.

## 8. Authorization and security

At minimum, management authorization separates：

- `fleet.read` and `fleet.write`；
- `template.read` and high-trust `template.publish`；
- independently grantable high-trust `template.attest`；
- `auth.read` and high-trust `auth.write`；
- each resource's retirement capability.

Template publication is equivalent to deploying reviewed code that can execute provider plugins with infrastructure credentials. Attestation independently asserts that one exact runtime subject passed the accepted suite, so `template.attest` MUST remain separately grantable from both `template.publish` and `fleet.write`. Auth writes disclose reusable GitHub credentials to Shaula, and Template publication may disclose sensitive platform bindings. All three capabilities require stronger controls than ordinary Fleet capacity changes.

The supported v1 Shaula listener binds loopback only; a configured non-loopback address fails startup. The reverse proxy terminates HTTPS, while Shaula itself owns mandatory OIDC verification, authorization and audit under spec 0009. Provider/client configuration is required through clap/env before startup. Every Profile/artifact API, including reads, uploads and attestation, requires an OIDC-derived session or verified API access token, even over direct loopback. Authentication precedes upload processing; cookie mutations also require Origin/CSRF validation. Legacy backend tokens and caller identity/scope headers cannot establish an actor. Native inbound HTTP TLS/mTLS remains outside v1. Loopback is not a tenant boundary.

Request bodies are never logged or attached to spans. Reverse-proxy access logs, body capture, panic dumps and tracing middleware must be tested against credential leakage. Profile GET/list/status/revision endpoints are redacted even for write-capable callers；rotation or binding replacement requires submitting new secret bytes.

The database ownership lock prevents a second Shaula writer but is not encryption or protection from a host administrator. Audit records store actor, resource, action, Revision and sanitized outcome；attestation audit also stores its key, canonical subject digest and suite version. Audit never stores credential or sensitive-binding bytes, Template archive contents or raw conformance output.

## 9. Failure and recovery

| Failure | Required behavior |
| --- | --- |
| HTTP response lost after commit | Retry resolves from idempotency without repeating validation/publication |
| Artifact body digest mismatch | Reject and remove only the temporary upload；create no artifact record |
| Artifact durable but Profile transaction absent | Retain as grace-period orphan, then GC after a reference check |
| Candidate validation interrupted | Lease expires and outbox/periodic scan resumes it |
| Template static validation rejected | Keep prior active Revision, never enter `Ready`, and expose a sanitized reason |
| Conformance result failed, mismatched or stale | Preserve the immutable audited attestation, keep the Candidate non-active and leave the prior active Revision selectable |
| Auth Candidate has bad syntax, identity or access | Classify separately as `CredentialMalformed`, `Unauthenticated`, `PermissionDenied` or `TargetHiddenOrNotFound`；keep prior active credential |
| GitHub returns a rate-limit response | Classify `RateLimited` from status/headers, honor the bounded retry time and do not misreport it as permission denial |
| Auth handoff cannot prove bound ownership/access | Mark the Fleet `Blocked`/degraded, retain old and desired tuple references, and never request-time fallback |
| Ordinary Fleet reconciliation cannot establish/replace the desired-tuple session | Keep acquisition stopped, preserve durable handoff/session references and retry without rolling back automatically |
| Persisted Scale Set lookup during handoff says missing | Record `ScaleSetMissing` without creating or rebinding；let ordinary Fleet reconciliation decide create-or-adopt |
| Decommission cleanup Auth handoff fails | Remain permanently non-acquiring, retain cleanup references and retry only cleanup work |
| SQLite commit fails | No accepted Revision/Change exists；do not run validators |
| Active Profile is requested for deletion | Stop new references and keep retirement Blocked without releasing desired/observed/in-flight/cleanup/recovery references |
| SQLite/artifact restore mismatch | Fail affected Profile/Fleet closed and preserve recovery evidence |
| OTLP exporter fails | Continue durable mutation/validation with bounded local diagnostics |

## 10. Day 0 observability

Required bounded spans include：

- `shaula.profile.registry.validate` and `shaula.profile.registry.commit`；
- `shaula.profile.change.reconcile`；
- `shaula.template.artifact.publish`, `shaula.template.validate`, `shaula.template.attestation.accept`, Template activation and artifact GC；
- `shaula.github_auth.validate`, `shaula.github_auth.handoff` and ordinary Fleet session reconciliation；
- `shaula.profile.reference.resolve`.

Metrics use finite `resource_kind=template_profile|github_auth_profile`, operation, state, `auth_kind=github_app|pat`, handoff state, result and stable reason code. Profile keys, Revision IDs, attestation IDs, artifact digests, GitHub owners/repositories, credential identity and error text are forbidden metric labels.

Secrets, request/response bodies, archive contents, Template binding values, private keys, PATs and secret-derived identifiers never appear in logs, spans or metrics. Local rate-limited diagnostics report exporter failure independently.

## 11. Acceptance criteria

Implementation is incomplete until：

1. A daemon starts with no `template_profiles` or `github_auth_catalog` bootstrap section, then creates both resource kinds through HTTP and recovers them after restart.
2. Artifact upload is digest-idempotent and rejects traversal, link/device entries, expansion bombs, content mismatch and oversize input without a published partial artifact.
3. Every effective Profile-spec PUT atomically commits Revision, Change, audit and outbox；every effective attestation/retirement mutation instead commits its endpoint-specific immutable record or state plus Change/audit/outbox, without creating or modifying a Template Profile Revision. All commit before returning, and no handler makes a Terraform/GitHub call.
4. Concurrent conditional writes have one winner；lost response retries return the original sanitized response.
5. Static validation can move a Template Candidate only to `Ready`；it cannot activate it, and a rejected replacement leaves the previous active Revision selectable.
6. An authorized `template.attest` request durably binds artifact, dependency lock, exact providers, engine binary, runner images, protected `bindings_digest`, runtime/trust policy with accepted limitations, manifest-derived `platform`/`bindings_contract` and suite version；only an exact passing subject gates a Template `Ready -> Active`, while Auth activation remains identity/access-validation gated.
7. A failed, mismatched or stale attestation remains audited and cannot activate or replace the previous active Revision；`template.publish`, `template.attest` and `fleet.write` are independently testable permissions.
8. Fleet admission accepts a bare or exact Template reference only when it denotes the current active Revision and pins its digest and attestation；later publication/attestation changes neither Fleet nor Generation.
9. PAT/App private-key bytes and sensitive Template binding bytes survive daemon restart from their immutable SQLite Revisions and reconstruct the exact client or IaC input, but never appear in any GET/list/status/revision/attestation response, audit, error, log, trace, metric or diagnostic representation；GitHub credentials never enter IaC/Runner, and Template binding secrets never enter Runner/workflow.
10. The Rust `shaula-scaleset` implementation passes the GitHub App/PAT × organization/repository matrix against real `github.com` and differential conformance against the fixed Go `github.com/actions/scaleset` oracle.
11. Wrong principal, App/installation mismatch, `401`, `403` and access-filtered `404` reject the Auth Candidate and never cause fallback or Scale Set Create.
12. Same-Profile promotion and admitted zero-occupancy cross-Profile replacement use one durable handoff: full desired/observed tuples, quiescence, ownership proof or unbound access classification, Busy Runner preservation and exact acknowledgements.
13. Handoff never creates/adopts, binds a Scale Set ID or establishes/replaces a session；ordinary Fleet reconciliation exclusively owns those effects while Target and Scale Set identity remain immutable.
14. Decommission permanently stops acquisition while allowing cleanup-only Auth handoff that cannot establish an acquiring session, create/adopt, rebind or mint JIT configuration.
15. An Auth Revision is GC-eligible only after desired/observed tuples, in-flight effects/sessions, Decommission cleanup and recovery references all clear；a `Blocked` state releases none of them.
16. Profile retirement refuses new references, remains visibly Blocked while in use and never deletes artifacts/credentials needed for Destroy or recovery.
17. Database/WAL/SHM/online-copy/backup/migration-copy/crash-dump tests and documentation identify every plaintext credential- or binding-bearing artifact and enforce restrictive permissions, retention and disposal.
18. OTel tests cover commit, validation, attestation/activation, Auth handoff, ordinary session reconcile and exporter failure without leaking bodies, bindings or credentials.

## 12. Open decisions

1. Which exact manifest-schema annotation marks a binding field sensitive, what normalized presence-only shape should mixed sensitive/non-sensitive bindings use in read responses, and which keyed/opaque non-verifier construction and encoding should `bindings_digest` use?
2. Resolved by ADR-0013 / spec 0009: mandatory OIDC sessions/API access tokens replace proxy actor assertions and backend authentication tokens.
3. Once an Auth Revision satisfies every reference-clearance rule and becomes GC-eligible, how long is its plaintext credential retained, and is explicit credential revocation part of retirement?
4. Should Profile DELETE remain asynchronous `Blocked(ResourceInUse)`, as specified, or return immediate `409` while referenced?
