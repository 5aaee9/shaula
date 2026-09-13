# Shared Template Pool Resource

- Status: Proposed.
- Decision: [ARD-0037](../ard/0037-shared-template-pool-resource.md).
- Extends: [spec 0029](0029-weighted-template-pool.md),
  [spec 0023](0023-fleet-template-follow-latest.md), and
  [spec 0002](0002-fleet-http-control-plane.md).
- Supersedes: the fleet-inline `template_pool` field defined by spec 0029 §2 for
  new Fleets. Fleets admitted under spec 0029 remain valid; see §6 migration.

## 1. Problem and boundary

Spec 0029 places the weighted member set inline inside the Fleet spec. That
binds a weighting policy to exactly one Scale Set: two Fleets that should draw
from the same backends must duplicate the member list, and the two copies drift
independently — editing weights or adding a route requires touching every Fleet
that carries the same pool.

This specification promotes the pool to a first-class **TemplatePool resource**,
a sibling of TemplateProfile. A Fleet references either a template
(`template_profile_ref`) or a pool (`template_pool_ref`), never both, and never
an inline `template_pool` for new admissions. Multiple Fleets may reference the
same TemplatePool, so a weighting policy is authored once and shared.

The boundary from spec 0029 §1 is unchanged: GitHub picks a Scale Set before
Shaula sees a job, so a shared pool cannot spread jobs across Fleets. Each
Fleet still owns one Scale Set and draws members within itself. Sharing a pool
shares the *member definition*, not the queue.

## 2. TemplatePool resource

A TemplatePool is a named, revisioned resource under
`/v1/template-pools/{key}` following the same lifecycle as TemplateProfile and
Fleet: create/replace via PUT with `If-None-Match: *`, replace via PUT with
`If-Match`, delete via DELETE with tombstone semantics.

```yaml
key: hkg-builders
members:
  - key: proxmox-hkg
    template_profile_ref: pve-mega-hkg-h3c   # bare key, follow latest Active
    weight: 20
    template_inputs: { ... }                # optional, bounded per member
    max_runners: 6                          # optional cap on this member
  - key: docker-tkyo
    template_profile_ref: proxmox-tyo-wanix
    weight: 10
failure_policy: backpressure                # or redistribute
```

- `members`: 1–32 entries; `key` unique stable identifier; `weight` integer
  1–10000. Same contract as spec 0029 §2.
- `template_profile_ref` is a **bare key that follows the profile's latest
  Active revision** (spec 0023 semantics). A member never pins a revision in
  its submitted form; the resolved pin exists only on the committed pool
  revision rows.
- `template_inputs` are bounded per member and validated against that member's
  Active revision input contract at pool admission.
- `failure_policy` is pool-global: `backpressure` (default) or `redistribute`.

## 3. Revision and resolution model

Each TemplatePool admission resolves every member's `template_profile_ref` to
its current Active TemplateProfile revision and commits **immutable member
rows** keyed by `(pool_key, pool_revision)`: `member_key`, resolved
profile/revision/artifact/attestation, inputs digest, and weight. The submitted
JSON is stored as the spec; resolved facts live on the member rows, mirroring
how Fleet revisions carry resolved template pins under spec 0029.

- A member row set is immutable for a pool revision. Editing members, weights,
  inputs, or policy mints a new pool revision.
- A pool revision change does not rewrite existing Generations. Generations
  keep the member pin they were admitted under.

## 4. Fleet reference and admission

- `FleetSpec.template_pool_ref: string` names a TemplatePool. It is mutually
  exclusive with `template_profile_ref`; exactly one of the two is required.
  The inline `template_pool` object is rejected for new Fleets (§6).
- A Fleet PUT that references a pool resolves the pool's **current revision**
  and records `(pool_key, pool_revision)` on the committed Fleet revision. This
  is the fleet's frozen routing context — the same role the resolved template
  pin plays for single-template fleets.
- Occupancy and ownership barriers are unchanged: changing which pool (or
  template) a Fleet references follows the existing zero-occupancy/replacement
  gate.

## 5. Level-triggered cascade for shared pools

Two cascades keep resolved pins fresh, both driven by the daemon scan tick and
both gated on the referencing Fleet's resource occupancy (spec 0023 §3):

1. **Template Active change → pool revision.** When a member's profile
   activates a new revision, the pool re-resolves that member and mints a new
   pool revision. Because the pool is shared, one profile activation produces
   exactly one new pool revision regardless of how many Fleets reference it.
2. **Pool revision change → fleet revision.** A referencing Fleet whose
   recorded `pool_revision` lags the pool's current revision is re-minted to
   the current revision once its occupancy is zero — the same deferred upgrade
   a single-template Fleet gets on a template Active change.

A pool revision is **eligible for admission** even while referencing Fleets are
occupied: the new revision exists and is selectable for *new* Fleets or *idle*
referencing Fleets; occupied Fleets catch up when they drain. There is no
pool-global occupancy gate, because the pool itself owns no runners — the
referencing Fleets do.

## 6. Capacity ceilings and the eligible draw set

Capacity is bounded at two levels:

- **Fleet `max_runners`** bounds the whole Fleet — the total Generations that
  Fleet may run regardless of which member drew them.
- **`member.max_runners`** bounds one member's Generations **across all Fleets
  referencing the same pool revision**. The cap is a property of the member's
  backend, not of one Fleet's queue, so it is enforced pool-wide inside the
  same atomic admission transaction that draws the member and inserts the
  Generation.

A member at its `max_runners` is **excluded from the eligible draw set**: the
weighted draw runs over members below their cap, with probabilities
renormalized across the eligible set (`weight[i]/Σeligible weight`). Exclusion
is observability-visible (the member's saturated state and the reduced eligible
set are reported), never a silent weight rewrite. When **no** member is
eligible — all at cap or unhealthy — admission backpressures: the Create is
deferred until capacity or health clears, and no Generation is admitted.

`failure_policy` governs *health/availability* failures, not cap exclusion:
`backpressure` (default) pauses Creates while a drawn member is unhealthy;
`redistribute` permits a redraw among the remaining eligible members. Cap
exclusion applies under both policies.

## 7. Deletion and retention

- Deleting a TemplatePool is rejected while any non-decommissioned Fleet
  revision references it. The check scans referencing fleet revisions in the
  same transaction as the tombstone write.
- Pool member rows are retained for as long as any live Generation or Fleet
  revision may resolve them; they are never garbage-collected while referenced,
  preserving spec 0029's "old Generations resolve their original member"
  guarantee.

## 8. Migration and non-goals

- Fleets admitted with the spec 0029 inline `template_pool` remain valid and
  keep their committed member rows. New Fleets must use `template_pool_ref`.
  Converting an inline-pool Fleet to a shared pool is a replace (new Fleet or
  a `template_pool_ref` change under the zero-occupancy gate), not an in-place
  rewrite of the member set.
- This does not share a queue across Fleets, does not add GitHub weight/priority
  APIs, and does not make independent Scale Sets one scheduling domain. A shared
  pool only shares the member definition and follow-latest resolution.

## 9. Acceptance outline

- A pool revision's member rows resolve to the members' current Active
  revisions at admission; the stored spec keeps bare `template_profile_ref`
  keys.
- Two Fleets referencing the same pool draw members independently per their own
  occupancy, while `member.max_runners` is enforced across both Fleets' combined
  Generations in one transaction.
- A member template's Active activation mints one new pool revision; each
  referencing Fleet catches up on its own zero-occupancy boundary, not
  simultaneously.
- Pool deletion is blocked while referenced; committed member rows stay
  resolvable for historical Generations.
- New Fleet PUTs reject an inline `template_pool` and accept
  `template_pool_ref`; existing inline-pool Fleets keep working unchanged.
