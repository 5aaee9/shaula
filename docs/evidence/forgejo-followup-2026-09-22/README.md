# Forgejo follow-up acceptance — 2026-09-22

These are **local real-platform** receipts, not simulated runtime or remote CI results.
The actual Shaula binary admitted profiles/Fleets over HTTP/OIDC, persisted SQLite,
and ran the unmodified bundled templates with Terraform 1.9.8. Reports were compared
as JSON against the fixture receipts; no raw credentials, SQLite, plans or logs are committed.

| Tuple | Evidence |
| --- | --- |
| Forgejo 16.0.4 / official Runner 13.1.0, exact OCI index in reports | Both runs |
| Docker Engine 29.6.2 / provider 3.0.2 | [Docker receipt](docker.json), 13 checks |
| Local Kind 0.32.0 / Kubernetes v1.36.1 / provider 2.33.0 | [Kubernetes receipt](kubernetes.json), 10 checks |

## Results and limits

- **A1 / A6:** success, failure and Busy daemon restart each retain one Generation,
  prove registration/resource absence and zero occupancy after reclaim. Real Jobs
  API retains the exact Task result; no Scale Set or verified Runner link is invented.
  Waiting/idle inventory is also exercised before sending a job to the retained runner.
- **A2:** two `runs-on` sets, one requiring an unavailable label: no Runner is created
  for the unmatched job; the matching job runs and reclaims while the other remains
  waiting. This corrected the previously reversed label-subset explanation in UI/spec.
  v15 source agrees: `Set.IsSubset(subset)` tests the argument against its receiver.
- **A3:** a positive waiting snapshot at zero capacity is retained unchanged through
  jobs-GET 503s and daemon restart. Raising capacity while stale does not Create;
  reducing another idle Fleet to zero does not authorize ordinary drain. Restored
  reads resume work. Hard expiry remains independent of stale demand.
- **A5, bounded real case:** discard a real 201 registration response. The undeclared
  runner lacks labels and cannot be claimed by name alone. Quarantine holds occupancy,
  performs no resource Create, and does not repeat POST across restart. Fixture server
  teardown removes the orphan; that is **not** successful automatic Shaula reclamation.
  Artificial None/ExactlyOne/Multiple branches remain local-test evidence, not a full
  real A5 classification matrix.
- **A7:** scan actual credentials against inputs, metadata, argv/env, runtime/daemon
  logs and authorized operation APIs, before and after reclaim. Docker has zero
  protected-plan/state token copies. Kubernetes recorded **two** token occurrences in
  the Destroy plan's exact bootstrap Secret data after provider refresh. The operator
  explicitly accepted this credential-grade plan/state/backup boundary; owner-only
  permissions and exclusion from all other scanned fields/logs are asserted. This is
  **not** a claim that Kubernetes tokens never enter Terraform. Bundled Forgejo v1
  does not emit Setup Info; its renderer/security checks remain local evidence.
- **Permissions:** four scoped management tokens support inventory/jobs/POST/GET/DELETE
  and Shaula Auth activation. Read-only tokens cannot mutate, other owners/private
  repositories and instance admin routes remain protected. Optional history requires
  repository access and `read:repository` unless already included. See [matrix](../../forgejo-permissions.md).
- **Hard-deadline recovery:** waiting/Busy resources are destroyed using the explicit
  lifetime policy. Docker DELETE and registration DELETE fail independently and are
  retried across restart without releasing occupancy early. Kubernetes proves Pod and
  immutable bootstrap Secret absence, not Docker-specific fault transport behavior.

## Bugs and environment findings

Real Kubernetes exposed kubectl's default `.kube/cache` inside the frozen Workspace
when HOME is cleared. That changed material after Create and blocked Destroy.
Both GitHub/Forgejo host bootstrap now explicitly use the existing private temporary
cache directory. A red/green unit test and the successful full Kubernetes rerun prove
the fix; the material commitment was **not** weakened to ignore the unexpected files.

Rootless Kind needed more host inotify watches during the test, with explicit owner
consent. The original **524288** was restored afterward; no persistent configuration
was written. The cluster could not resolve its image registry, so a digest-preserving
OCI archive was preloaded after verifying the original index SHA-256. Provider mirrors
retained readonly lock validation. No custom Runner image/template was substituted.
The initial stale-demand fixture had overlapping prefixes; making them non-overlapping
fixed the fixture without weakening production ownership checks.

After both successful runs, the isolated Kind cluster was deleted, the private Engine
had no remaining containers or volumes and was stopped, and temporary Nix roots were
removed. No system Docker/Podman service or existing cluster was reconfigured or used
to host test workloads. Bootstrap explicitly selected the real Docker CLI on PATH;
the first attempt with the system-compatible CLI had failed and was not counted.
Protected failed/successful fixture evidence remains local, not in this repository.

## Still separate

Safe early idle fencing (A4), full real A5 classification coverage, real GitHub A8,
VM/cloud boot/destruction, weighted-load distribution, and other supported Forgejo
versions are **not** established by these receipts. No acquisition proxy was added.
Final local Rust/browser verification is recorded in [Implementation Status](../../IMPLEMENTATION_STATUS.md),
not promoted into external deployment acceptance. Reproduce with the [harness](../../../scripts/forgejo-lifecycle/README.md).
