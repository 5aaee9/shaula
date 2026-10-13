# GitLab lifecycle acceptance

Real `shaula serve`, a disposable GitLab CE, the bundled Docker template and
real Terraform. Nothing is simulated. It covers spec 0043 §10 G1 (closed
loop), part of G2 (a job with an extra tag stays pending), G7 (job ↔ owned
registration while running) and G8 (leak scan).

```sh
# Inside a delegated cgroup (see .github/workflows/gitlab-e2e.yml):
nix develop --command bash scripts/gitlab-lifecycle-acceptance.sh
```

Requirements: a real Linux Docker Engine (not Podman) on
`unix:///var/run/docker.sock`, `DOCKER_MIN_API_VERSION=1.24` for the pinned
provider, and a `target/debug/shaula` build. GitLab CE needs about 4 GiB of
memory and 3–5 minutes to boot.

## Flow

1. Boot GitLab on the Docker bridge gateway, so both the daemon (host) and
   Generation containers (default bridge) use the same `external_url`. Create
   one private group and project, plus an Owner group access token with
   `create_runner`, `manage_runner` and `read_api`.
2. Publish the Docker template with `runner_backend: gitlab`. Create the
   `gitlab_token` auth profile and the Fleet with the actual `shaula` CLI
   (`auth-profiles create`, `fleets create`).
3. Check that the paused anchor exists and that zero demand creates nothing.
4. Trigger one pipeline: `match` (Fleet tag) and `mismatch` (Fleet tag + `gpu`).
5. Check:
   - exactly one Generation container, running the official image as `gitlab-runner`;
   - the job claimed by the owned registration (ID, description and `maintenance_note`);
   - the registration paused while the job runs;
   - job success, with the output in the job trace;
   - `mismatch` still pending.
6. Check reclaim:
   - occupancy returns to 0 and the Generation is `Destroyed`;
   - the registration is gone (404);
   - the container and both anonymous volumes are removed.
7. Retire the Fleet: tombstone (410), and no anchor or Generation registration
   remains in the group.

The leak scan runs while the job is running and again after Destroy. Neither
the group token nor the registration token may appear in:

- APIs, the daemon log or Operation Logs;
- retained Terraform inputs or state;
- container metadata;
- the job trace.

The runner's own log may show its short token prefix. No Shaula-owned surface
may.

The report prints to stdout, and to `$SHAULA_GITLAB_REPORT` when set. It names
only checks, versions and digests. Teardown removes only the GitLab server ID
and containers carrying this run's random Fleet key.
