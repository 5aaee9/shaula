# GitLab lifecycle E2E (2026-10-13)

## What this proves

Actual `shaula serve` drives one demand-driven Generation against a disposable
GitLab CE through the bundled Docker template and real Terraform. Nothing is
simulated. Covered by [spec 0043](../../specs/0043-gitlab-runner-backend.md) §10:

- **G1** closed loop: paused anchor → Create → claim → pause → finish → pause →
  recheck → delete → Destroy, then Fleet retirement deletes the anchor.
- **G2** partial: a job with an extra `gpu` tag stays `pending`.
- **G7** partial: `job.runner` is the owned registration (ID, description and
  `maintenance_note`). The Jobs projection does not exist yet.
- **G8**: neither the group access token nor the registration token appears on
  11 surfaces while the job runs or on 10 after Destroy. These are APIs, the
  daemon log, Operation Logs, retained Terraform inputs and state, container
  metadata and the runner log. The job trace is checked separately for both
  full tokens.

Not covered: G3–G6, Kubernetes and VM templates, and multiple concurrent jobs.

## Environment

GitHub Actions run
[38401342029](https://github.com/5aaee9/shaula/actions/runs/38401342029),
`ubuntu-24.04`, commit `b41b292`. GitLab CE 19.4.2, Docker Engine 28.0.4 and
Terraform 1.9.8. The runner image is
`gitlab/gitlab-runner:v19.4.1@sha256:77d03a5c…` pinned by digest. The binary,
artifact and provider-lock digests are in [`results.json`](results.json).

The Fleet and its `gitlab_token` auth profile were created with the actual
`shaula` CLI (`auth-profiles create`, `fleets create`). Reproduce with
`scripts/gitlab-lifecycle-acceptance.sh`; see
[its README](../../../scripts/gitlab-lifecycle/README.md).

## Findings

- gitlab-runner writes its short token (the 9-character prefix of the
  registration token) into its own log and into the header of every job trace
  (`on <runner> <short token>`). This is upstream behaviour, so the report
  records it as `traceShowsUpstreamShortToken: true` instead of failing. Shaula
  surfaces must not show the short token, and none did.
- The first CI attempt failed because a cold image pull wrote progress output
  before the container ID. The harness now pre-pulls the image and reads the
  last line.
