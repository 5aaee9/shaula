---
status: proposed
date: 2026-09-12
---

# Route one Scale Set through a weighted Template Pool

GitHub selects a Scale Set before Shaula sees a job and does not expose a
configurable cross-Scale-Set assignment algorithm. We therefore keep the v1
Fleet contract (one Scale Set and one homogeneous Template) and define a
future Template Pool: one Scale Set receives the queue, while Shaula chooses
among immutable Template Profile routes at acquisition time with durable
weighted scheduling. The Pool must distinguish request-bound assignment (only
when the provider contract proves it) from runner-mix mode for pre-created idle
Runners.

Adding `weight` to the two existing Fleets was rejected because it would only
change local capacity or runner pre-creation. The other Fleet would still not
see jobs assigned to the first Scale Set, and labels cannot reliably reroute
queued work. A Pool also avoids claiming that GitHub itself guarantees a 1:2
job ratio: weights are a best-effort long-run policy for eligible acquisitions,
with explicit capacity, failure and spillover semantics. Runner-mix mode
reports inventory composition only; it cannot be presented as per-job routing.

The change is intentionally a new resource model. It requires a Pool revision
and route schema, request-to-route acquisition facts, per-route capacity and
health accounting, Generation/runtime pin propagation, API/UI changes, and a
drain migration from the current Fleets. Until those pieces and real GitHub
acceptance exist, the proposed design must not be represented as implemented.
