---
status: accepted
date: 2026-09-13
---

# Promote the weighted template pool to a shared resource

Spec 0029 embedded the weighted member list inside the Fleet spec, which made a
weighting policy private to one Scale Set. The user asked to manage pools
separately — to let a Fleet choose a pool *or* a template, with pool and
template as sibling resources, and to let one pool be shared across Fleets.
The user also asked for members to follow the latest Active template revision
rather than pin.

We promote the pool to a named, revisioned `TemplatePool` resource under
`/v1/template-pools`, parallel to TemplateProfile and Fleet. A Fleet references
a pool by `template_pool_ref` (mutually exclusive with `template_profile_ref`);
the inline `template_pool` field is dropped for new Fleets. Members keep the
spec 0029 shape — key, bare `template_profile_ref`, weight, bounded inputs,
optional cap — but `template_profile_ref` is now a bare key that **follows the
profile's latest Active revision** under spec 0023, resolved to concrete member
rows at pool admission and stored as immutable `(pool_key, pool_revision)`
records.

Sharing raises two questions that inline pools never had. First, propagation:
a member's Active activation must mint one new pool revision, and each
referencing Fleet re-mints to the current pool revision on its own
zero-occupancy boundary — a two-stage, level-triggered cascade, not a global
synchronous push. Occupied Fleets lag deliberately; a pool is never blocked by
an occupied Fleet because the pool owns no runners. Second, caps:
`member.max_runners` is a property of the member's backend, so it is enforced
**pool-wide** — across all referencing Fleets' combined Generations — inside
the same atomic transaction that draws the member and inserts the Generation.
`failure_policy` is likewise evaluated per drawing Fleet against shared
occupancy.

Alternatives considered and rejected:

- **Keep inline pools, add a copy/reference mechanism.** Duplicating member
  lists per Fleet preserves spec 0029 exactly but reintroduces drift: two Fleets
  that should share a policy would each carry a private copy. Rejected because
  the user's requirement is sharing one definition, not convenient duplication.
- **Shared pool with synchronous propagation.** Forcing all referencing Fleets
  onto a new pool revision at once would couple their occupancy and create a
  fleet-wide blast radius on every member change. Rejected in favor of the
  existing deferred, per-Fleet cascade used by spec 0023.
- **Per-Fleet member caps.** Capping each Fleet independently would let two
  Fleets together oversubscribe a member's real backend capacity. Rejected;
  the cap protects the backend, so it is counted across the shared pool.

Implications:

- A new top-level resource (table, revisions, immutable member rows keyed by
  `(pool_key, pool_revision)`), `/v1/template-pools` CRUD with If-Match /
  If-None-Match / tombstone, and a `template_pool_ref` field + pool↔template
  mutual exclusion on Fleet.
- The generation admission path resolves `pool_member_key` against
  pool-revision member rows rather than fleet-revision rows; occupancy and the
  member cap are counted across referencing Fleets.
- A second cascade (template→pool→fleet) joins the existing template→fleet
  cascade, both occupancy-gated and level-triggered.
- Pool deletion is reference-checked; member rows are retained for historical
  Generations.

Implementation and real GitHub acceptance are tracked separately from this
accepted decision.
