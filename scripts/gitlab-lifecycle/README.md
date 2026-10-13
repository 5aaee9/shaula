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

## Fault suites (G3–G6)

```sh
SHAULA_GITLAB_SUITE=demand nix develop --command bash scripts/gitlab-lifecycle-acceptance.sh
```

`SHAULA_GITLAB_SUITE` selects one suite per run (`baseline`, the default, is
`run.mjs` above). Each suite boots its own GitLab and writes its report to
`$SHAULA_GITLAB_REPORT`. `demand`, `busy` and `uncertain` point the auth
profile and every Fleet at `faults.mjs`: a proxy on the Docker bridge gateway
in front of GitLab. Generation runners use it too, but runner job traffic is
never faulted. The proxy never reads a response body (registration responses
carry a token). From requests it keeps only the registration fields (no
credential); it never keeps headers. `node --test` covers its fault modes.

- **`demand` (G3).** A `max_runners: 0` Fleet keeps a positive snapshot. Each
  case first brings up a Fleet with one Idle Generation. Then it injects the
  fault:
  - GraphQL demand query rejected (503);
  - both anchors deleted, with anchor re-creation rejected;
  - both anchors unpaused, with anchor re-creation rejected;
  - the token's bot member demoted to Maintainer.

  The check waits for `Degraded`, then lowers capacity to 0 (a new revision).
  For 50 s the Idle Generation, its container and registration must remain,
  and both `fleet_demand` rows must keep their value and proof time. Ticks
  must keep running (inventory requests counted). An unpaused anchor must be
  deleted, never re-paused. After restore, demand is proven again, the
  Generation is retired, and each Fleet has exactly one paused anchor for its
  current revision. Every replaced anchor returns 404.
- **`busy` (G4).** An Idle Generation is selected for Retirement by a capacity
  change. The proxy holds that Retirement `PUT paused=true` while the
  still-unpaused runner claims a new job, then lets it through. The check
  expects:
  - the Generation becomes Busy, with `cleanup.job_busy`;
  - container and paused registration are kept, and the job keeps running,
    through the revision change and a Fleet DELETE;
  - after the job succeeds: registration 404, Destroy, tombstone.
- **`token` (G5).**
  - A copy of the Docker template with `restart = "always"` is published. Its
    Fleet must never create a container; the Generation settles in cleanup and
    the Fleet retires cleanly.
  - After claim and pause, a second `gitlab-runner run-single` is `docker
    exec`ed on the Generation's own `config.toml`, with stdio discarded. The
    second pending job must stay pending for the full wait timeout. Then a new
    Generation's registration serves it.
- **`uncertain` (G6).** Registration responses are lost:
  - anchor response dropped after commit: adopted, one POST;
  - anchor response held while an identical anchor is registered out of band:
    both deleted, one rebuilt;
  - Generation request reset before GitLab: nothing registered, Generation
    Destroyed without a resource, a new Generation serves the job;
  - Generation response dropped after commit: `CleanupRequired`, registration
    404, retry serves the job;
  - Generation response held while an identical duplicate is registered:
    `Quarantined`, both kept, no replay, no Create.

  Every case also asserts that the uncertain POST was never replayed. The
  quarantined Fleet is deliberately not retired; GitLab teardown owns its two
  registrations.
