# GitLab lifecycle acceptance

Real `shaula serve`, a disposable GitLab CE, the bundled Docker or Kubernetes
template and real Terraform. Nothing is simulated. It covers spec 0043 §10 G1
(closed loop), part of G2 (a job with an extra tag stays pending), G7 (the Jobs API
shows the Verified association, which survives registration deletion) and G8
(leak scan).

```sh
# Inside a delegated cgroup (see .github/workflows/gitlab-e2e.yml):
nix develop --command bash scripts/gitlab-lifecycle-acceptance.sh
# Kubernetes template, against a disposable Kind cluster:
SHAULA_ACCEPTANCE_BACKEND=kubernetes KUBECONFIG=/path/to/kind-kubeconfig \
  SHAULA_ACCEPTANCE_KUBE_CONTEXT=kind-shaula-gitlab \
  nix develop --command bash scripts/gitlab-lifecycle-acceptance.sh
```

Requirements: a real Linux Docker Engine (not Podman) on
`unix:///var/run/docker.sock`, `DOCKER_MIN_API_VERSION=1.24` for the pinned
provider, and a `target/debug/shaula` build. GitLab CE needs about 4 GiB of
memory and 3–5 minutes to boot. Kubernetes additionally needs `kubectl` and
a Kind cluster on that Engine whose context name starts with `kind-shaula-`;
the run creates and deletes one namespace named after its random prefix.

## Flow

1. Boot GitLab on the Docker bridge gateway, so both the daemon (host) and
   Generation containers (default bridge) use the same `external_url`. Create
   one private group and project, plus an Owner group access token with
   `create_runner`, `manage_runner` and `read_api`. On Kubernetes, GitLab is
   published on loopback only and an in-process TCP relay listens on the
   bridge gateway: Kind Pods reach host-local addresses (the Forgejo Kubernetes
   path relies on the same route), whereas another bridge's DNAT target may be
   isolated. The `external_url` stays identical for the daemon and the Pods.
2. Publish the platform template with `runner_backend: gitlab` (Kubernetes also
   binds the run's namespace and kubeconfig). Create the `gitlab_token` auth
   profile and the Fleet with the actual `shaula` CLI (`auth-profiles create`,
   `fleets create`).
3. Check that the paused anchor exists and that zero demand creates nothing.
4. Trigger one pipeline: `match` (Fleet tag) and `mismatch` (Fleet tag + `gpu`).
5. Check:
   - exactly one Generation runner, running the official image as `gitlab-runner`:
     - Docker: one container, no restart policy, exactly the image's two
       anonymous volumes;
     - Kubernetes: one Pod as UID 999, `restartPolicy: Never`, no
       service-account token, no init container, only the bootstrap Secret item
       and the `emptyDir` home; the Secret is immutable with only `config.toml`,
       and `run-single` persisted `.runner_system_id` in the writable home;
   - the job claimed by the owned registration (ID, description and `maintenance_note`);
   - the registration paused while the job runs;
   - the Jobs API (`jobs.mjs`) shows a `backend: gitlab` record, running or
     queued, whose `generations` holds the Verified Generation;
   - job success, with the output in the job trace;
   - `mismatch` still pending.
6. Check reclaim:
   - occupancy returns to 0 and the Generation is `Destroyed`;
   - the registration is gone (404);
   - Docker: the container and both anonymous volumes are removed;
     Kubernetes: the exact Pod and Secret are gone.
   - the Jobs record still holds that association, now `Destroyed`, and the
     job's own `success` (`report.checks.jobs`); GitLab's `job.runner` after
     deletion is recorded as `upstreamRunnerAfterDelete`.
7. Retire the Fleet: tombstone (410), and no anchor or Generation registration
   remains in the group.

The leak scan runs while the job is running and again after Destroy. Neither
the group token nor the registration token may appear in:

- APIs (including Jobs), the daemon log or Operation Logs;
- retained Terraform inputs or state;
- container or Pod metadata, and Secret metadata;
- the job trace.

The runner's own log (`docker logs` or `kubectl logs`) may show its short token
prefix. No Shaula-owned surface may. As for Forgejo, the Kubernetes provider's
refresh may keep the released `config.toml` in the original bootstrap Secret's
retained state data; the scan accepts it only there, with the Secret's exact
name, namespace and UID.

The report prints to stdout, and to `$SHAULA_GITLAB_REPORT` when set. It names
only checks, versions and digests. Teardown removes only the GitLab server ID,
containers carrying this run's random Fleet key and, on Kubernetes, the exact
namespace the run created.
