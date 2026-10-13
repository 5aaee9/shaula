# GitLab pool backend protocol experiment

Runs official GitLab CE **19.4.2** and gitlab-runner **19.4.1** in disposable
containers on a private network. It verifies the upstream facts that
[spec 0043](../../docs/specs/0043-gitlab-runner-backend.md) relies on, **not**
Shaula Pool/Terraform integration or the G1–G9 acceptance contract.

Requirements: Linux, Node.js >= 22, a **local** Docker or Podman engine, about
6 GiB free memory, and the two official images. GitLab needs 3–5 minutes to boot.
No existing GitLab URL, account or registration is used. The jobs are private,
generated `shell` executor jobs containing `echo` and `sleep`.

```sh
podman pull docker.io/gitlab/gitlab-ce:19.4.2-ce.0
podman pull docker.io/gitlab/gitlab-runner:v19.4.1
CONTAINER_ENGINE=podman node scripts/gitlab-pool-spike/verify.mjs > results.json
# Alternatively: CONTAINER_ENGINE=docker with a local Docker engine.
```

The admin token prepares fixtures only (group, project, pipelines, a foreign
instance runner). Every Shaula-side call uses an Owner group access token with
`create_runner`, `manage_runner` and `read_api`. Runner tokens reach containers
only as a copied `config.toml`, never argv or environment. The report contains
image IDs, versions and booleans/counts; no credential is emitted. Output is
written only after every assertion and fixture teardown succeeds.

Cases:

1. `demand`: a paused, never-contacted anchor registration used as
   `compatibleRunnerId` counts only the matching job (not the extra-tag or
   untagged job). Without the filter a non-admin sees 0 jobs; on a foreign
   instance runner it also sees 0 **without an error** while the runner may still
   be visible, so `userPermissions.updateRunner` is the authorization proof.
2. `closedLoop`: `run-single --max-builds 1` picks the job, `job.runner.id`
   equals the registration, the runner reports `active`, the process exits 0 and
   the registration **remains** until deleted. Leak scan of argv/env/logs.
3. `noJob`: with no compatible job, `--wait-timeout` also exits 0.
4. `pause`: a paused registration does not pick a pending job; pausing while a
   job runs neither interrupts it nor clears the association.
5. `deleteWhileRunning`: deleting a busy registration returns 204, the job still
   succeeds, and `job.runner` becomes `null`.
6. `tokenReuse`: two processes sharing one token run two jobs concurrently.
7. `identity`: duplicate descriptions are accepted; `maintenance_note` round-trips.

Cleanup removes only container IDs, anonymous volumes and the network created by
this run; it never prunes the engine.

## Adapter E2E

`adapter-e2e.mjs` reuses the same fixture to run the Rust test
`crates/shaula-gitlab/tests/live_gitlab.rs` (ignored by default) against the
disposable instance. Secrets reach the test only as 0600 files named in the
`SHAULA_GITLAB_E2E` JSON. The artifact is printed after the test and teardown
succeed; see [the evidence](../../docs/evidence/gitlab-adapter-e2e-2026-10-13/README.md).

```sh
CONTAINER_ENGINE=podman node scripts/gitlab-pool-spike/adapter-e2e.mjs > results.json
```

Killing the script mid-run skips teardown. In that case remove the run's
`shaula-gl-<id>` containers, their volumes and network by exact ID or name.
