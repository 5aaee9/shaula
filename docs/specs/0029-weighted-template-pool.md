# Weighted Template Pool

- Status: Accepted for implementation (2026-09-12); verification and provider acceptance are tracked separately.
- Decision: [ARD-0036](../ard/0036-weighted-template-pool.md).
- Extends: [spec 0001](0001-shaula-runner-scale-set.md), [spec 0002](0002-fleet-http-control-plane.md), and [spec 0004](0004-template-profile-runtime.md).

## 1. Problem and boundary

GitHub chooses a Scale Set before Shaula receives a `JobAvailable` message. Two
independent Fleets with the same label therefore cannot be weighted by Shaula;
the unselected Fleet never observes that request and GitHub provides no
cross-Scale-Set reroute operation.

This specification defines a **Template Pool** specification shape on the
Fleet resource: one Fleet owns one GitHub Scale Set and a weighted set of
Template members. New Fleets select either `template_profile_ref` or
`template_pool`, never both. Existing single-template Fleets keep their
behavior; a pool is created as a new Fleet rather than converting either of
the existing same-label Scale Sets in place.

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

## 3. Independent weighted random selection

Each new Generation independently draws one Template member with probability
`P(member i) = weight[i] / sum(weight)`. With weights 10 and 20, each draw has
probabilities 1/3 and 2/3. The implementation MUST NOT guarantee exact counts
in any batch, window or cumulative history. Consecutive selections of the same
member are valid. Weighted round-robin, deficit scheduling, quotas and
compensation based on historical deviation are outside this implementation.

The selected member and its exact Template/input pins are committed atomically
with Generation admission before JIT or IaC effects. Restart, retry and cleanup
reuse that choice; an existing Generation is never redrawn. A later, distinct
Generation makes a new independent draw. No scheduler cursor or historical
deficit is needed to implement the probability contract.

GitHub `acquire_jobs` accepts request IDs and returns accepted IDs; JIT minting
is a separate operation without a request-to-Runner binding. This implementation
therefore controls which Template creates a Runner, not which job GitHub sends
to that Runner. It MUST NOT persist an invented job-to-member association.
Actual Jobs associations continue to require observed Runner identity.

Listener persist-before-ACK and acquisition remain unchanged; weights MUST NOT
filter pending request IDs or provide a selective NACK/delay mechanism.
`backpressure` is the default failure policy: when a member is unavailable or
at its optional cap, pause new pool Creates until the condition clears.
`redistribute` explicitly permits random selection among eligible members with
probability proportional to their weights; the reduced eligible set and reason
must be observable. Neither policy withholds acquisition messages or cancels
already admitted Generations. No policy silently changes weights.

## 4. Capacity and observations

The Pool keeps aggregate GitHub `TotalAssignedJobs` for its Scale Set and
per-route effective capacity, occupancy, admitted Generations and failures.
The scheduler must preserve the Pool's global `max_runners` while distributing
new Creates according to eligible route weights. Metrics and Jobs views must
show configured weights and observed route counts separately; observed counts
are diagnostic and never treated as GitHub's routing truth.

Random member selection, capacity reservation and Generation insertion are
committed in the same short transaction. A Create/JIT/Apply failure keeps
occupancy and its route facts according to the existing cleanup/quarantine
rules; it must not silently reassign that Generation. Redistribution is an
explicit Pool policy with an auditable reason.

Scaling down never retires a Runner merely to correct a random proportion.
Only global excess, health and normal lifecycle policy justify retirement, and
the existing Busy-safe removal gate remains authoritative.

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

- Deterministic sampling tests cover each weighted interval, 10:20 probabilities,
  single-member selection and invalid/zero/overflowing inputs. Seeded large
  samples may check a broad statistical range, never an exact 1:2 count.
- Concurrent admissions preserve global/member capacity and commit exactly one
  member pin per Generation. Restart/retry retains that pin, while fresh
  Generations remain independent random draws.
- Duplicate delivery, ACK/acquire uncertainty and stale epochs retain the
  existing listener guarantees without inventing request-to-Generation binding.
- Route capacity limits, health failures and spillover/backpressure are
  durable, observable and recoverable.
- A Pool revision change cannot alter existing Generations or bypass occupancy,
  ownership and template retirement barriers.
- Integration evidence demonstrates that two old same-label Fleets still have
  arbitrary GitHub assignment, while a single Pool is the only path on which
  Shaula's weighted scheduler operates.
