# Shaula v1 Profile HTTP Control-Plane Specification

- Status: Draft
- Date: 2026-09-04
- Managed resources: Template Profiles and GitHub Auth Profiles
- Persistence: SQLite, including content-addressed Template archive bytes; reconstructable filesystem execution cache
- Secret-at-rest decision: PAT, GitHub App private key, and schema-sensitive Template bindings may be plaintext in SQLite

This specification extends the [Fleet HTTP Control-Plane Specification](0002-fleet-http-control-plane.md). Related decisions are [ADR-0005](../ard/0005-manage-fleet-desired-state-through-http-and-sqlite.md), [ADR-0007](../ard/0007-use-target-bound-github-auth-profiles.md), [ADR-0009](../ard/0009-manage-profile-resources-through-http-and-sqlite.md), and [ADR-0013](../ard/0013-require-openid-connect-for-all-http-access.md). Inbound authentication is normative in [spec 0009](0009-mandatory-openid-connect.md); GitHub Auth Profiles remain outbound credentials, not OIDC identities.

## 1. Outcome

One `shaula serve` HTTP Interface manages three desired-resource families：

- Fleet；
- Template Profile；
- GitHub Auth Profile.

`template_profiles` and `github_auth_catalog` are not daemon-bootstrap truth sources. Their desired heads, immutable Revisions, conformance attestations, status, Changes, idempotency and audit facts live in SQLite. Template bytes are uploaded through HTTP and stored as immutable digest-addressed archives in SQLite; the protected filesystem artifact store is a reconstructable execution cache. Default filesystem source imports and Terraform variable discovery follow [spec 0015](0015-template-library-and-variable-discovery.md). GitHub PAT/App private-key bytes and schema-sensitive Kubernetes/Docker binding values are accepted as write-only HTTP fields and stored in their immutable SQLite Revision rows.

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

Template Artifact storage is a separate Module because archive integrity, safe extraction and reconstructable execution caches differ from Profile revision mutation; both durable archives and Profile rows now use SQLite under spec 0015. GitHub credential bytes leave the Registry only through an internal scoped handoff to the GitHub Access Module；they never cross Fleet or Template Runtime Interfaces, never enter a Terraform input and never enter a Runner. Sensitive Template bindings leave the Registry only through an exact-Revision handoff to the approved IaC child；they never enter a Runner or workflow.

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
- `Ready` is Template-only and means built-in static validation passed and automatic activation is pending；new references wait for `Active`.
- A still-current Template Candidate automatically advances `Ready -> Active` under spec 0017, without a conformance request；an Auth Candidate instead needs successful identity/access validation for `Validating -> Active` and never consumes a Template attestation.
- The current `active_revision` is the only Revision that may receive a new Fleet reference, including an explicit exact-revision reference.
- A rejected or superseded Candidate never replaces the previous active Revision.
- An already-admitted Fleet's exact Template reference remains usable for normal reconciliation, future Create, Destroy and recovery during controlled retirement until the Fleet releases it.

Every Template/Auth Profile-spec PUT uses a strong ETag, `If-None-Match`/`If-Match`, a bounded `Idempotency-Key`, an authenticated actor and strict JSON. An effective Profile-spec mutation atomically commits an immutable Revision, Profile Change, sanitized audit fact and outbox marker before returning `202`. Attestation PUT and Profile DELETE use the same conditional/idempotent discipline but commit their endpoint-specific immutable record or retirement state plus Change/audit/outbox；they never create or modify a Template Profile Revision. No HTTP handler invokes Terraform or GitHub. An identical valid Profile-spec PUT returns replayable `200` without a new Revision or Change.

Profile GET and mutation `202` / no-op `200` responses also return `Shaula-Resource-Version`, following the [Fleet conditional-write contract](0002-fleet-http-control-plane.md). Its quoted value equals the origin ETag and remains usable for `If-Match` when a compression proxy weakens the representation ETag. Clients preserve the version captured when the edit opened, prefer this header, and fall back only to a strong ETag; weak or stale validators never authorize a write.

For a mutation containing a GitHub credential or sensitive Template binding, the idempotency record points at the committed Auth or Template Revision. A retry compares any supplied secret only in protected memory with that revision and returns its sanitized response；the record contains no reusable body or secret-derived verifier. Reusing the key with different content returns conflict. Profile DELETE is safe retirement, not row deletion. It immediately rejects new references and waits for live consumers: Fleet exact Template/attestation pins, desired/observed Auth tuples, effects/sessions, validators, workers/Generations, cleanup and recovery. `Blocked` is not reference-clearance evidence. Section 7.1 defines the explicit release of the retiring Profile's own heads and separates runtime references from retained history; a Profile must not deadlock retirement on its own historical revision numbers. There is no force delete.

## 4. HTTP Interface

All paths are below `/api/v1`：

| Method | Path | Meaning |
| --- | --- | --- |
| `PUT` | `/template-artifacts/sha256:{digest}` | Idempotently upload an immutable Profile archive |
| `GET` | `/template-artifacts/sha256:{digest}` | Authorized metadata only, not artifact download |
| `GET` | `/template-sources` | Stored default source catalog, under spec 0015 |
| `GET` | `/template-artifacts/sha256:{digest}/variables` | Exact artifact Terraform variable discovery, under spec 0015 |
| `GET` | `/template-profiles` | Paginated Profile summaries |
| `PUT` | `/template-profiles/{profileKey}` | Create or publish a Candidate Revision |
| `GET` | `/template-profiles/{profileKey}` | Desired/active metadata and ETag |
| `GET` | `/template-profiles/{profileKey}/status` | Validation and reference status |
| `GET` | `/template-profiles/{profileKey}/revisions/{revision}` | One immutable sanitized Revision |
| `GET` | `/template-profiles/{profileKey}/revisions/{revision}/input-contract` | Bounded visual Fleet-input projection under [spec 0014](0014-visual-template-inputs.md) |
| `DELETE` | `/template-profiles/{profileKey}` | Retire after all references clear |
| `PUT` | `/template-profiles/{profileKey}/revisions/{revision}/attestations/{attestationKey}` | Submit one immutable conformance result |
| `GET` | `/template-profiles/{profileKey}/revisions/{revision}/attestations/{attestationKey}` | Read bounded attestation metadata |
| `GET` | `/github-auth-profiles` | Redacted Auth inventory; collection shape and UI contract in [spec 0012](0012-github-authentication-inventory.md) |
| `PUT` | `/github-auth-profiles/{profileKey}` | Create or rotate a Candidate Auth Revision |
| `GET` | `/github-auth-profiles/{profileKey}` | Redacted desired/active metadata and ETag |
| `GET` | `/github-auth-profiles/{profileKey}/status` | Validation, rollout and reference status |
| `GET` | `/github-auth-profiles/{profileKey}/revisions/{revision}` | One immutable redacted Revision |
| `DELETE` | `/github-auth-profiles/{profileKey}` | Retire after dependent Fleets clear |
| `GET` | `/profile-changes/{changeId}` | Query asynchronous validation/activation/retirement |

Artifact upload declares digest and exact compressed/content length before the body is accepted. Shaula bounds the compressed body, verifies digest and safe archive shape in isolated temporary storage, commits the immutable original archive to SQLite, and materializes its execution cache before acknowledging success. Repeated uploads reuse the same content. Path traversal, links/reparse entries, devices, oversized expansion and duplicate normalized paths are rejected. Upload does not create, attest or activate a Template Profile Revision. Variable declaration consistency and legacy artifact compatibility follow spec 0015.

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
The wire field named `bindings_digest` is an opaque server-issued commitment to the exact immutable binding Revision. It MUST NOT be an unkeyed digest of sensitive plaintext or otherwise permit offline validation of a guessed secret. It may appear in protected envelopes and attestation subjects only under that non-verifier property；otherwise it is secret and must be redacted. Representation and compatibility remain decision D4 in the [central register](../README.md#仍需决定或冻结); this change does not introduce a new commitment format or rewrite old rows.



The versioned `shaula-profile` manifest inside the artifact is the sole authority for normalized `platform` and `bindings_contract`. Shaula derives and exposes those values after parsing the artifact；the PUT body, binding names and attestation cannot override them. A Docker artifact therefore declares Docker and `shaula.bindings.docker/v1` in its manifest and may expose a trusted `docker_host` binding schema；the Profile PUT supplies only a value allowed by that schema.

A Profile incarnation fixes its manifest-derived `platform` and `bindings_contract`. Changing from Kubernetes to Docker, or to an incompatible bindings contract version, requires a new Profile key. Compatible artifact, binding or bounded-input-policy changes create Candidate Revisions under the same key.

Asynchronous static validation：

1. Re-open the already published artifact by digest.
2. Verify archive containment, manifest version, manifest-derived `platform`/`bindings_contract`, declared inputs/outputs, provider lock checksums and engine constraints.
3. Reject undeclared files, missing lock file, arbitrary executable hooks, Profile-owned backend configuration/overrides, reserved worker-file collisions, a caller-supplied platform authority and Profile/Fleet input confusion according to policy.
4. Retain publication-time engine, protected-binding and Fleet-input-policy admission checks. This activation scan does not run Terraform apply, call GitHub or create a Runner; actual engine/platform conformance remains separate evidence.
5. If this Candidate is still desired and all checks pass, automatically transition it through `Ready` to `Active` in the fenced transaction defined by [spec 0017](0017-automatic-template-activation.md). The same revalidation path processes existing Ready candidates after upgrade; rejected or retired candidates cannot replace the prior Active.

### 5.1 Independent conformance evidence

Template publication authorizes automatic activation after static validation under spec 0017. A trusted external harness separately tests the exact runtime subject, and an authorized caller may submit durable immutable conformance evidence such as：

```json
{
  "result": "passed",
  "subject": {
    "artifact_digest": "sha256:...",
    "dependency_lock_digest": "sha256:...",
    "providers": [
      {"source": "hashicorp/kubernetes", "version": "x.y.z", "checksums_digest": "sha256:..."}
    ],
    "engine": {"kind": "terraform", "required_version": ">= 1.9, < 2.0", "version": "x.y.z", "binary_digest": "sha256:..."},
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

No attestation result changes a Profile head or activation ID. Passed, failed, mismatched and stale attestations all remain durable and audited independently of activation. Automatic `Ready -> Active` freezes a separate `static-validation-v1:` opaque activation provenance ID and immutable activation audit in one transaction. Legacy `active_attestation_id`, `template_attestation_id`, `attestation_id` and related wire names carry that provenance while retaining historical conformance IDs unchanged; their presence does not imply a passing runtime suite. Spec 0017 owns this compatibility contract.

A Fleet may submit a bare Profile key or exact `{key, revision}`. A new or changed reference must resolve the current `active_revision`; an unchanged existing pin in a capacity/inputs update or no-op is not new-reference admission. Spec 0002 §4.1 owns that distinction. An already-admitted Fleet may keep using an older retained exact pin for normal reconciliation, future Create, Destroy and recovery until it releases that reference. Publishing or attesting a newer Revision never changes an existing Fleet/Runner；switching a Fleet follows the zero-Resource-Occupancy rule.

#### Trust and evidence

The v1 integrity boundary is authenticated OIDC submission, independent `template.attest` authorization, exact subject validation and append-only audit/immutable attestation storage. The recorded actor is the server-verified `(iss, sub)`, never a body claim. A separately signed envelope/PKI is not required; a caller with publication/Fleet permission alone cannot attest. The daemon does not rerun the conformance harness or independently prove arbitrary platform assertions.

`evidence_digest` identifies a retained, bounded **sanitized** conformance report, not a hash of raw state, credentials or provider output. The trusted harness records the exact tested tuple, suite/version, scenario results and accepted limitations. Operator audit must be able to locate the corresponding protected report; an arbitrary caller-supplied `passed` flag without this trust/authorization boundary is not evidence. Infrastructure administrators and the authorized attester remain in the trusted computing base.

The typed subject includes the engine's `required_version` constraint as well as resolved version/binary digest. Unknown/duplicate JSON fields are rejected; equality is against the Registry-derived exact subject, not independent platform assertions. Canonical serialization/digest and request replay use the versioned subject codec and fixed cross-restart golden vectors, never raw request property order. Array order remains part of canonical submitted representation; key/content changes conflict rather than rewriting the record. Provider/image set membership must still match the expected tuple. Digest/codec changes require explicit versioning and compatibility tests; D4's unresolved bindings representation is not silently replaced here.

### 5.2 Binding sensitivity and read shape

For v1 `schemas/bindings.schema.json`, `properties.<field>.sensitive` is the boolean annotation. A true value protects the entire top-level member/subtree; omitted annotation is conservatively sensitive, and a non-boolean annotation is invalid. Nested mixed-secret objects are protected as a whole, not split through JSON-path read exceptions. Publishers must not mark credential-bearing material non-sensitive.

The normalized v1 read projection is deliberately small: omit `bindings` values altogether and expose only `bindings_present: boolean` (whether any binding material was supplied). It does not expose per-field lengths, hashes, names derived from values or a new map of secret fingerprints. This conservative projection also omits non-sensitive binding values; richer mixed-field reads are not promised. It applies to Profile GET/list/revision/attestation-related reads regardless of caller write permission. A full replacement must resubmit required bindings, not round-trip redacted placeholders or infer deletion from omitted response fields.

## 6. GitHub Auth Profile resource

Proposed amendment: [spec 0011](0011-multi-account-github-authentication.md) / [ADR-0015](../ard/0015-route-one-github-app-profile-to-multiple-accounts.md) defines a versioned multi-account GitHub App policy and installation-routing model. Until accepted, the single-installation identity and rotation-only rules below remain the baseline. PAT and common Profile lifecycle rules are not replaced by that proposal.

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

The following identity fields are immutable for one Auth Profile incarnation：kind, App ID, installation ID, initial authenticated PAT principal and normalized Target allowlist. Changing any of them requires a new Profile key and explicit Fleet reassignment. A Fleet Auth-key replacement follows spec 0002's zero-Occupancy/effect barrier; an existing idle session is quiesced by the subsequent Handoff, not an admission deadlock. Target and Scale Set identity remain immutable. A replacement PUT on the existing key is credential rotation only：new App private-key bytes for the same App/installation, or a new PAT that authenticates as the same GitHub principal. A PAT whose principal cannot be proved equal is a principal migration, not rotation.

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
- explicit Fleet-to-Template-Revision-and-activation-provenance relations (retaining legacy attestation field names under spec 0017) plus `fleet_auth_handoffs` containing desired/observed Auth `(profile_key, revision)` tuples, fences, classifications and exact references held by in-flight effects, sessions, Decommission cleanup and recovery.

Auth Revision rows contain PAT/App private-key plaintext, and Template Revision rows contain plaintext values for schema-sensitive bindings. SQLite main/page files, WAL/SHM, online and migration copies, crash dumps and backups MUST receive credential-grade access, retention and disposal. Application-level encryption is not a v1 requirement；deployment-level full-disk/filesystem/backup encryption is strongly recommended.

The authoritative consistency set is defined by [spec 0010 §7](0010-lifecycle-worker-and-http-state-backend.md): SQLite including Terraform state/locks/worker facts, retained artifact/inputs and unresolved emergency state. Ordinary materialized copies are reconstructible. Partial restore is unsupported; no external secret-store restore is required for the credential bytes kept in SQLite.

### 7.1 Retirement completion and reference release

1. DELETE atomically marks the incarnation Retiring, advances its mutation fence, records Change/audit/outbox and closes new-reference admission/publication/activation. Already-pinned consumers keep the exact materials needed for normal reconciliation and cleanup; retirement does not mutate or silently upgrade them.
2. Validators/activation attempts must recheck that fence before committing and release their own claims after completion or verified cancellation. Pending candidate work cannot promote after retirement starts.
3. Recompute **live execution references** transactionally: current Fleet pins/heads, non-terminal Changes that still need materials, Auth tuples, in-flight sessions/effects, worker/Generation/cleanup/recovery claims. An old immutable Revision, terminal Change, audit record or tombstone retained only for history is not automatically an execution reference.
4. While any live consumer remains, retain the needed Profile heads/material and expose `Blocked(ResourceInUse)`. Verified consumer progress may release its references; merely labelling a Change Blocked or deleting a history row is not proof.
5. When all external execution references and validator/activation claims clear, one transaction rechecks the retirement fence and releases the Profile's own desired/active/observed **execution-head references**, records Retired/tombstone and completes its retirement Change. Last-revision/high-water numbers may remain as explicitly historical metadata; they no longer authorize execution or keep retirement permanently Blocked.
6. Physical GC is later and independent: retain audit, immutable metadata, credential/artifact/state bytes for their applicable retention periods and any remaining verified recovery need. Tombstone/ETag/idempotency replay remain valid. Credential revocation timing is D2; retirement does not revoke externally while an in-flight/cleanup consumer still needs it.

Retirement and new-reference/activation transactions use the same admission fence: exactly one side wins. A crash before step 5 retries from Retiring; a crash after its commit replays completion without releasing any newly referenced incarnation. History retention never authorizes a new Fleet reference or a second worker.

## 8. Authorization and security

At minimum, management authorization separates：

- `fleet.read` and `fleet.write`；
- `template.read` and high-trust `template.publish`；
- independently grantable high-trust `template.attest`；
- `auth.read` and high-trust `auth.write`；
- each resource's retirement capability.

Template publication is equivalent to deploying reviewed code that can execute provider plugins with infrastructure credentials, and authorizes automatic activation after validation. Attestation independently asserts that one exact runtime subject passed the accepted suite, so `template.attest` MUST remain separately grantable from both `template.publish` and `fleet.write`; it is not required for activation. Auth writes disclose reusable GitHub credentials to Shaula, and Template publication may disclose sensitive platform bindings. All three capabilities require stronger controls than ordinary Fleet capacity changes.

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
| Conformance result passed, failed, mismatched or stale | Preserve the immutable audited attestation without changing Profile head or activation provenance |
| Auth Candidate has bad syntax, identity or access | Classify separately as `CredentialMalformed`, `Unauthenticated`, `PermissionDenied` or `TargetHiddenOrNotFound`；keep prior active credential |
| GitHub returns a rate-limit response | Classify `RateLimited` from status/headers, honor the bounded retry time and do not misreport it as permission denial |
| Auth handoff cannot prove bound ownership/access | Mark the Fleet `Blocked`/degraded, retain old and desired tuple references, and never request-time fallback |
| Ordinary Fleet reconciliation cannot establish/replace the desired-tuple session | Keep acquisition stopped, preserve durable handoff/session references and retry without rolling back automatically |
| Persisted Scale Set lookup during handoff says missing | Record `ScaleSetMissing` without creating or rebinding；let ordinary Fleet reconciliation decide create-or-adopt |
| Decommission cleanup Auth handoff fails | Remain permanently non-acquiring, retain cleanup references and retry only cleanup work |
| SQLite commit fails | No accepted Revision/Change exists；do not run validators |
| Active Profile is requested for deletion | Close new references; wait for live consumers, then atomically release self heads and finish under §7.1; history retention alone cannot deadlock it |
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
5. Static validation automatically moves the still-current Candidate through `Ready` to `Active` with durable activation provenance; rejected replacements leave the previous active Revision selectable. Existing Ready candidates use the same revalidation path after upgrade, and repeated scans are idempotent.
6. An authorized `template.attest` request durably binds artifact, dependency lock, exact providers, engine binary, runner images, protected `bindings_digest`, runtime/trust policy with accepted limitations, manifest-derived `platform`/`bindings_contract` and suite version as independent conformance evidence；it does not gate Template activation. Auth activation remains identity/access-validation gated.
7. Passed, failed, mismatched and stale attestations remain audited and cannot activate or replace any Profile head/activation ID；`template.publish`, `template.attest` and `fleet.write` are independently testable permissions.
8. New/changed Fleet Template references resolve current Active and pin its digest/activation provenance; unchanged pins in capacity/inputs/no-op requests are retained under spec 0002. Later publication, activation or attestation changes neither Fleet nor Generation.
9. PAT/App private-key bytes and sensitive Template binding bytes survive daemon restart from their immutable SQLite Revisions and reconstruct the exact client or IaC input, but never appear in any GET/list/status/revision/attestation response, audit, error, log, trace, metric or diagnostic representation；GitHub credentials never enter IaC/Runner, and Template binding secrets never enter Runner/workflow.
10. The Rust `shaula-scaleset` implementation passes the GitHub App/PAT × organization/repository matrix against real `github.com` and differential conformance against the fixed Go `github.com/actions/scaleset` oracle.
11. Wrong principal, App/installation mismatch, `401`, `403` and access-filtered `404` reject the Auth Candidate and never cause fallback or Scale Set Create.
12. Same-Profile promotion and admitted zero-occupancy cross-Profile replacement use one durable handoff: full desired/observed tuples, quiescence, ownership proof or unbound access classification, Busy Runner preservation and exact acknowledgements.
13. Handoff never creates/adopts, binds a Scale Set ID or establishes/replaces a session；ordinary Fleet reconciliation exclusively owns those effects while Target and Scale Set identity remain immutable.
14. Decommission permanently stops acquisition while allowing cleanup-only Auth handoff that cannot establish an acquiring session, create/adopt, rebind or mint JIT configuration.
15. An Auth Revision is GC-eligible only after desired/observed tuples, in-flight effects/sessions, Decommission cleanup and recovery references all clear；a `Blocked` state releases none of them.
16. Profile retirement rejects new references, preserves live cleanup/worker material, and eventually releases its own heads when consumers/validators clear. Tests cover unused Active profiles, historical revisions/terminal Changes, consumer release, activation/admission races and crash/replay before/after the Retired transaction.
17. Database/WAL/SHM/online-copy/backup/migration-copy/crash-dump tests and documentation identify every plaintext credential- or binding-bearing artifact and enforce restrictive permissions, retention and disposal.
18. OTel tests cover commit, validation, attestation/activation, Auth handoff, ordinary session reconcile and exporter failure without leaking bodies, bindings or credentials.
19. Trusted attestation tests cover absent/forged identity, wrong scopes, exact subject/engine constraints, evidence linkage, canonical replay and immutable audit; no unimplemented independent signature verification or daemon-run harness is claimed.
20. Binding schema annotation tests cover true/false/omitted/invalid sensitivity and nested secret material; every read uses §5.2's presence-only projection without placeholders, secret fingerprints or accidental round-trip updates.

## 12. Open decisions

The [central decision register](../README.md#仍需决定或冻结) owns D2 (retention and external credential revocation timing) and D4 (commitment representation/compatibility). Asynchronous retirement, separate attestation authority, sensitive-field read rules and OIDC management authentication are contract decisions, not competing implementation options.
