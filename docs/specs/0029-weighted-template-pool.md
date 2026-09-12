# Weighted Template Pool

- Status: Proposed (2026-09-12); implementation and provider acceptance are not complete.
- Decision: [ARD-0036](../ard/0036-weighted-template-pool.md).
- Extends: [spec 0001](0001-shaula-runner-scale-set.md), [spec 0002](0002-fleet-http-control-plane.md), and [spec 0004](0004-template-profile-runtime.md).

## 1. Problem and boundary

GitHub chooses a Scale Set before Shaula receives a `JobAvailable` message. Two
independent Fleets with the same label therefore cannot be weighted by Shaula;
the unselected Fleet never observes that request and GitHub provides no
cross-Scale-Set reroute operation.

This specification defines a future **Template Pool** resource. A Pool owns
one GitHub Scale Set and a weighted set of Template Profile Revisions. It is a
new resource shape, not a `weight` field added to an existing Fleet. The
existing v1 contract remains one Fleet, one Scale Set and one Template until
this specification is accepted and implemented.

## 2. Pool contract

- A Pool has one GitHub Target, runner group, label set, Auth Revision Ref and
  Scale Set identity.
- A Pool revision contains one or more immutable routes. Each route pins one
  exact Template Profile Revision, its own bounded approved inputs and input
  digest, and a positive integer weight. Route pins are resolved and committed
  atomically at admission; differing member schemas are validated separately.
- The durable revision record stores route members separately from the
  submitted JSON (`member_id`, exact profile/revision/artifact/attestation,
  inputs digest and weight). This preserves reference retention and lets old
  Generations resolve their original member after later Pool changes.
- A Generation continues to pin exactly one route and one Template Profile
  Revision for its entire lifecycle. Template publication never mutates an
  existing Generation.
- The route set is immutable for a Pool revision. Changing weights, routes or
  route inputs creates a new Pool revision and follows the existing zero
  occupancy/replacement barriers.

## 3. Acquisition and scheduling

The Pool listener persists each positive `runner_request_id` before ACK. It
must send the complete pending request set to `acquire_jobs`; it may not use
weight as a selective NACK or delay mechanism. A request-bound implementation
may allocate a route using durable smooth weighted round-robin (or equivalent
deficit) state and store that choice with the acquisition intent before or
atomically with acquisition; recovery and uncertain outcomes must reuse the
exact choice. The resulting JIT material and Generation record then retain the
request-to-route binding.

If the provider contract or an existing idle Runner does not prove that a job
will land on the selected Generation, the Pool must use **runner-mix mode**:
weights control the construction of eligible Runner inventory only. Shaula
must report this as best-effort runner composition and must not call it a
per-job ratio guarantee.

Weights are a long-run scheduling target for eligible acquisitions. They do not
override GitHub's Scale Set selection, do not reroute an already assigned job,
and do not promise an exact ratio for a finite batch. A route that is at its
capacity, unhealthy or blocked is temporarily ineligible; the Pool must expose
the reason and apply an explicit operator-selected policy: bounded spillover
to another eligible route or backpressure. Silent weight renormalization is
not allowed.

## 4. Capacity and observations

The Pool keeps aggregate GitHub `TotalAssignedJobs` for its Scale Set and
per-route effective capacity, occupancy, pending acquisitions and failures.
The scheduler must preserve the Pool's global `max_runners` while distributing
new Creates according to eligible route weights. Metrics and Jobs views must
show configured weights and observed route counts separately; observed counts
are diagnostic and never treated as GitHub's routing truth.

Scheduler reservations and cursor/deficit state are committed in the same
short transaction as Generation admission. A Create/JIT/Apply failure keeps
occupancy and its route facts according to the existing cleanup/quarantine
rules; it must not silently reassign the failed request. Redistribution is an
explicit Pool policy with an auditable reason.

## 5. Migration and non-goals

Existing Fleets cannot be converted in place by adding a field. Migration must
create a new Pool Scale Set, validate all route revisions, drain and safely
retire the old Fleets, then switch workflows to the Pool label. Keeping the old
Scale Sets online with the same label during migration retains GitHub's
arbitrary assignment race.

This specification does not promise strict 1:2 job placement, does not add a
GitHub weight/priority API, and does not make two independent Scale Sets a
single queue. Strict routing remains a workflow or external-dispatcher
responsibility using distinct labels.

## 6. Acceptance outline

- Repeated acquisitions with all routes eligible converge to the configured
  weighted target within a documented tolerance; runner-mix mode is measured
  as inventory composition, while request-bound mode is measured as accepted
  acquisition assignments.
- Restart, duplicate delivery, ACK/acquire uncertainty and stale session
  epochs preserve one request-to-route decision where the provider proves that
  binding, and never create a second Generation for the same request.
- Route capacity limits, health failures and spillover/backpressure are
  durable, observable and recoverable.
- A Pool revision change cannot alter existing Generations or bypass occupancy,
  ownership and template retirement barriers.
- Integration evidence demonstrates that two old same-label Fleets still have
  arbitrary GitHub assignment, while a single Pool is the only path on which
  Shaula's weighted scheduler operates.
