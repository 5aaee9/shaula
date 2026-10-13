# GitLab acceptance G1–G8 (2026-10-14)

GitHub Actions run
[38453877721](https://github.com/5aaee9/shaula/actions/runs/38453877721),
commit `41f284c`, `ubuntu-24.04`. Every job uses a disposable GitLab CE 19.4.2,
actual `shaula serve`, real Terraform 1.9.8, and the official
`gitlab/gitlab-runner:v19.4.1` image pinned by digest. Docker Engine is 28.0.4;
Kubernetes is Kind v0.33.0 (server v1.37.0). Nothing is simulated. Faults are
transport faults from a proxy between the daemon and GitLab
(`scripts/gitlab-lifecycle/faults.mjs`), plus out-of-band changes made with the
GitLab admin API or Rails. Each report holds only checks, versions and digests;
`glpat-`/`glrt-` do not appear in any of them.

| Spec 0043 §10 | Report | What was observed |
| --- | --- | --- |
| G1 closed loop, G2 partial, G7, G8 (Docker) | [`baseline-docker.json`](baseline-docker.json) | anchor → Create → claim → `Verified` → pause → success → delete → Destroy → retire; `gpu` job stays pending; leak scan of 13 surfaces while the job runs and 12 after Destroy |
| same, Kubernetes | [`baseline-kubernetes.json`](baseline-kubernetes.json) | one Pod as UID 999, `restartPolicy: Never`, no service-account token; the immutable Secret holds only `config.toml`; Pod and Secret gone after Destroy; leak scan of 14 / 12 surfaces |
| G7 Jobs view | both baseline reports, `checks.jobs` | `backend: gitlab` record `running`, linked to the Verified Generation; after the registration is deleted (GitLab `job.runner` = `null`) the link is kept, the Generation is `Destroyed`, and the record shows `success` / `confirmed` |
| G3 demand snapshot | [`demand.json`](demand.json) | demand poll rejected, anchor deleted, anchor unpaused, token bot demoted to Maintainer: each case gives `Degraded` with the demand row byte-identical, and the Idle Generation, its container and its registration kept for 20 passes through a capacity change. Once demand is proven again, the Generation is retired and drifted anchors are replaced (404) |
| G4 Busy-safe | [`busy.json`](busy.json) | a Retirement pause raced by a claim: Busy, resources kept through a revision change and a Fleet DELETE; the 180 s job succeeds, then delete → Destroy → tombstone |
| G5 token reuse | [`token.json`](token.json) | `restart = "always"` is refused at saved-plan admission, before any container exists; a second `run-single` on the same `config.toml` ran 33 s while paused and the other job stayed pending (16 polls), then a new Generation served it |
| G6 registration Uncertain | [`uncertain.json`](uncertain.json) | anchor ExactlyOne adopted with one POST; anchor Multiple: both deleted, one rebuilt; Generation None: zero resources, retry succeeds; Generation ExactlyOne: `CleanupRequired`, registration deleted, retry succeeds; Generation Multiple: `Quarantined`, both kept, no replay or Create |
| Adapter | [`adapter.json`](adapter.json) | `jobRead`: `job.runner` names the registration while running, `null` with `success` after deletion |

## Findings fixed while getting here

- **Unpaused anchor** (`8ab0916`): it was paused again in place. §3.1 calls
  this drift, so it is now replaced.
- **Uncertain ExactlyOne** (`8ab0916`): the single candidate was accepted
  without checking its tags and `run_untagged`. It must now match them too.
- **Hidden `maintenance_note`** (`41f284c`): a demoted token still lists and
  reads runners, but GitLab returns `maintenance_note: null` and refuses
  `PUT` with 403. A local probe on 19.4.2 showed this. Readiness had treated
  the hidden note as foreign ownership and quarantined the Idle Generation.
  It now waits, the removal gate reports Unavailable, and the anchor record is
  left alone (spec §3 updated).
- **Fixture:** the members API refuses to change a token bot's role (403),
  so G3 demotes the bot through Rails.

## Limits

- **G3 in CE:** the GraphQL `updateRunner=false` path, with the inventory
  still readable, needs an EE custom role and is not exercised.
- **G5:** the restart policy is caught when the saved plan is admitted, not
  when the template is published, because HCL is not evaluated statically.
- **VM templates:** these have only the local guest-bootstrap evidence in
  [gitlab-vm-bootstrap-2026-10-14](../gitlab-vm-bootstrap-2026-10-14/README.md).
  There is no real cloud VM run.
