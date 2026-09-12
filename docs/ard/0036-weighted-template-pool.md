---
status: accepted
date: 2026-09-12
---

# Route one Scale Set through a weighted Template Pool

GitHub selects a Scale Set before Shaula sees a job and does not expose a
configurable cross-Scale-Set assignment algorithm. We therefore keep the v1
single-template Fleet behavior and add a Template Pool shape for new Fleets:
one Scale Set receives the queue, while Shaula independently samples a Template
member when admitting each new Runner Generation. Pool members must all be
suitable for every job matched by that Scale Set's labels and trust policy.

The user chose **weighted randomness, not exact distribution**. Weights 10/20
mean probabilities 1/3 and 2/3 for each new Generation. No finite batch or
cumulative total must match 1:2; streaks are valid. We reject weighted
round-robin, historical deficit compensation and ratio-driven retirement.
Persist the chosen member with the Generation so retry/restart never redraws
an existing identity; no scheduling cursor is required.

Adding `weight` to the two existing Fleets was rejected because it would only
change local capacity or runner pre-creation. The other Fleet would still not
see jobs assigned to the first Scale Set, and labels cannot reliably reroute
queued work. A Pool also avoids claiming that GitHub itself guarantees a 1:2
job ratio: acquisition returns accepted request IDs and JIT is a separate
Scale Set operation, so weights control Runner Template selection only.
Capacity and failure policies explicitly define whether Creates pause or
redistribute among eligible members; the listener's ACK/acquire path is unchanged.

The new specification shape reuses Fleet ownership, authentication, revision
and HTTP boundaries with mutually exclusive single-template and pool forms.
It requires immutable member rows, per-member capacity/health accounting,
atomic random selection and Generation insertion, API/UI changes and drain
migration from the current Fleets. Implementation and real GitHub acceptance
must be reported separately from this accepted decision.
