# GitLab adapter E2E (2026-10-13)

## What this proves

The `shaula-gitlab` adapter, called only through the core `GitlabPoolPort`
(the same way the daemon uses it), drives one complete Generation against a
real GitLab. Covered: auth probe, scope pinning, anchor demand, registration,
`config.toml` delivery, claim, pause, finish, and removal. It supports
[spec 0043](../../specs/0043-gitlab-runner-backend.md) §3–§6. It is **not** the
G1–G9 acceptance run. The daemon supervisor, Terraform templates, fault
injection of lost registration responses, VM and Kubernetes are out of scope.

## Environment

- Local Linux amd64, rootless Podman. Official GitLab CE **19.4.2** and
  gitlab-runner **19.4.1**; the images are in [`results.json`](results.json).
- A fresh disposable instance on a private network, with one private group
  (`fleet`) and project (`app`). Every adapter call used an Owner **group access
  token** (`create_runner`, `manage_runner`, `read_api`). The admin token only
  created fixtures, triggered the pipeline and edited the anchor out of band.
- The pipeline has `match` (`[shaula-e2e]`) and `mismatch` (`[shaula-e2e, gpu]`).
  The CI file was committed with `[skip ci]`, so only the explicit pipeline
  creates demand.

Reproduce with:

```sh
CONTAINER_ENGINE=podman node scripts/gitlab-pool-spike/adapter-e2e.mjs > results.json
```

The script prints the artifact only after the Rust test
(`crates/shaula-gitlab/tests/live_gitlab.rs`) passes and fixture teardown succeeds.

## Results

| Step | Observed |
| --- | --- |
| Probe | version `19.4.2`, pinned group id matches, zero runners |
| Scope pinning | the same path with a different expected id → `PermissionDenied` |
| Idle demand | paused anchor proves `0` |
| Pending demand | `1`; the `gpu` job is not compatible |
| Anchor unpaused out of band | demand **unproven** (`None`), not `0`; re-paused through the port |
| Generation before start | `never_contacted`, not paused |
| Claim | `running_jobs` = the `match` job in `fleet/app` |
| Pause while busy | `Done`; the job keeps running |
| Finish | process exit 0, job `success`, `running_jobs` empty |
| Registration after exit | first read `status=paused`, `job_execution_status=idle` (lag 0 s) |
| Leak scan | token absent from `inspect`, container logs and the job trace |
| Removal | delete `Done` then `AlreadyAbsent`; pause on a gone runner `AlreadyAbsent`; inventory empty |

## Findings fed back

- A paused registration reports `status: "paused"`. The adapter originally
  treated this as an unknown status, and the run timed out waiting for the
  registration to become idle. `GitlabRunnerRef::is_known` now accepts it.
- `run-single` streams job output to GitLab (the job trace), not to the
  container log. The output check reads the trace through the API.
- An earlier attempt, where the CI-file commit triggered a push pipeline, showed
  that idle demand is only meaningful with no unrelated pipelines in scope.
