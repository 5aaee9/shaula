# GitLab pool backend: upstream protocol experiment (2026-10-13)

## Decision supported by this experiment

**GitLab can serve Shaula's Pool model with a stronger association level than
Forgejo, provided demand comes from a Fleet-owned paused anchor registration
with an explicit permission proof, and Shaula itself enforces one job per
registration by pausing after acquisition.** This supports
[ARD-0043](../../ard/0043-admit-gitlab-as-the-second-pool-backend.md) /
[spec 0043](../../specs/0043-gitlab-runner-backend.md); it does not accept them.

## Real environment and scope

- Local Linux amd64, rootless Podman; official GitLab CE **19.4.2**
  (`ecff27ae760`) and gitlab-runner **19.4.1**. Image IDs are in
  [`results.json`](results.json).
- A fresh disposable GitLab on a private container network; one private group
  and project. The pipeline has three jobs: `match` (`[shaula-spike]`),
  `mismatch` (`[shaula-spike, gpu]`) and `untagged`. Jobs only `echo` and
  `sleep 20` with the `shell` executor.
- Every Shaula-side call used an **Owner group access token** with
  `create_runner`, `manage_runner`, `read_api`. The admin token only prepared
  fixtures and the foreign instance runner.
- **Not a Shaula Rust Pool/Terraform integration test**, not G1–G9 acceptance,
  no fault injection of lost registration responses, no instance scope, no
  Kubernetes, no VM. Resource removal used the local engine CLI.

Reproduce with [`scripts/gitlab-pool-spike`](../../../scripts/gitlab-pool-spike/README.md).

## Results

All seven cases passed; output was written only after teardown succeeded.

| Case | Observation | Consequence for spec 0043 |
| --- | --- | --- |
| `demand` | Paused, never-contacted anchor: `compatibleRunnerId` count = 1 (`match` only). Non-admin without the filter: 0. Non-admin on a foreign instance runner: **0, no error, runner still visible**, `updateRunner=false`; admin sees 1 | Demand requires the anchor plus a same-request `userPermissions.updateRunner` proof (§3) |
| `closedLoop` | `run-single --max-builds 1` from a copied `config.toml`; `job.runner.id` = registration; runner `active` while busy; exit 0; registration **remains** (`online`/`idle`) after exit | `Verified` association; Shaula must delete the registration (§4, §6) |
| leak scan | Token absent from argv, env and logs; logs contain the 9-char short token, a **substring of the full token** | Runner logs are credential-adjacent (§5) |
| `noJob` | `--wait-timeout 10` with no compatible job: exit 0 after 13 s, 0 jobs | Exit code is not cleanup evidence (§4) |
| `pause` | Paused before start: process picks nothing, job stays pending. Paused while running: job continues to `success`, association kept | Pause is the drain primitive (§6) |
| `deleteWhileRunning` | DELETE busy registration → 204; job keeps running and succeeds; `job.runner` becomes `null` | Deletion does not interrupt jobs but erases association; persist first (§6) |
| `tokenReuse` | Two processes with one token ran **2 jobs concurrently** on one registration | No restart policies; pause after acquisition (§4, §5) |
| `identity` | Duplicate descriptions accepted (2 exact matches); `maintenance_note` round-trips | Ownership by joint evidence; `Multiple` → Quarantine (§2, §6) |
| `mismatchStillPending` | 10 non-matching jobs left pending at the end | Tag matching is enforced by GitLab |

An earlier run of the same harness failed because an instance runner created
without `run_untagged` defaulted to `true` and also matched the untagged job;
the final harness sets it explicitly. This confirms `run_untagged` is part of
server-side matching and must be pinned by Shaula (§2).

## Not established

- Minimum GitLab version for `compatibleRunnerId` and its cost on large
  instances (D7 / R5).
- Behaviour when the registration response is lost (G6), stale-runner cleanup
  of a never-contacted anchor, and token expiry policies.
- The race between `paused=true` and an in-flight job request; spec 0043 waits
  one inventory interval and re-checks before deleting, which is not yet proven.
