# Forgejo lifecycle acceptance

This harness starts **actual `shaula serve`**, isolated SQLite and a disposable HTTPS OIDC issuer. It publishes the bundled artifact and credential through authenticated HTTP; the production supervisor and Terraform own Runner Create/Destroy. No fake runtime or direct Runner start/registration substitutes for the lifecycle. Permission-only probes separately create/delete registrations to test scoped API authorization.

Prerequisites: Linux, real Docker Engine (not Podman), Node 22.13+, OpenSSL, tar, Terraform **1.9.8**, and a built Shaula binary. The caller must be in the Docker host network namespace (enter the RootlessKit child user/mount/network namespace for rootless Docker). Only generated test workflows run. Do not expose fixture ports on a production host.

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --locked -p shaula
export DOCKER_HOST=unix:///var/run/docker.sock
export TERRAFORM_BIN=/absolute/path/to/terraform
# Optional DOCKER_BIN and SHAULA_BIN are exact executables.
# The real docker and kubectl must also be first on PATH: runtime bootstrap
# resolves these host CLIs independently of the harness's DOCKER_BIN.
node --test scripts/forgejo-lifecycle/*.test.mjs
node scripts/forgejo-lifecycle/run.mjs
```

Docker 29 may need `DOCKER_MIN_API_VERSION=1.24` for provider 3.0.2; change only an owned disposable Engine. Rootless DNS may require a provider filesystem mirror, with the original readonly provider lock. Do not modify templates or weaken plan admission to make a test pass.

## Docker checks

- Success/failure and daemon restart during Busy: one ephemeral registration/Generation, exact Task result in Jobs, then Destroyed, zero occupancy and no container/anonymous credential volume/registration. Jobs associations remain Unverified.
- Matching and mismatched `runs-on`: a missing requested label leaves the job waiting and does not create a Runner. The job's requested labels must be a subset of Runner labels, not the reverse.
- Jobs management GET returns 503: positive demand and its timestamp survive restart, no ordinary Create/drain occurs. Restore polling, finish jobs, reclaim normally. Fleet prefixes are non-overlapping.
- Four minimum permission categories, public Shaula Auth activation, read-only mutation denial, wrong-owner/instance denial and the additional repository read permission for optional history.
- `max_lifetime_secs: 15`: reclaim waiting and active resources. Hard expiry still works with failed demand reads. Independently fail Docker DELETE and registration DELETE, restart at each checkpoint, retain occupancy until both converge. These are **not idle-safe drain** tests.
- Lose a real successful registration response: the undeclared runner has no joint label ownership proof, so quarantine with occupancy held; no repeated POST, no resource Create. This does **not** prove that name-only ownership is safe, or cover all artificial ExactlyOne/None/Multiple classifications on a live server.
- Check actual per-Runner and management credentials against argv/env, metadata, runner/daemon logs, Jobs/Generation/Operation Log APIs, and Terraform inputs. Decode saved plans rather than grep compressed bytes. Bundled Forgejo v1 templates do not emit Setup Info; its rendering/redaction remains covered by local tests.

Fault proxies only alter management jobs reads, registration responses and DELETE outcomes. Acquisition traffic passes unchanged. No production acquisition proxy is introduced.

## Local Kubernetes

Provision an **isolated, disposable Kind cluster** yourself. The harness creates/deletes only its random namespace and requires an explicit context beginning `kind-shaula-`; it never uses an ambient/default cluster.

```sh
export SHAULA_ACCEPTANCE_BACKEND=kubernetes
export KUBECONFIG=/absolute/path/to/disposable-kubeconfig
export SHAULA_ACCEPTANCE_KUBE_CONTEXT=kind-shaula-forgejo-followup
node scripts/forgejo-lifecycle/run.mjs
```

Terraform uses the existing Kubernetes template/provider **2.33.0**. Assertions require one non-root Pod with restartPolicy Never, no service-account token, one host-released immutable Secret, and eventual Pod/Secret/registration absence. Success/failure, Busy restart, labels, stale demand, waiting/Busy hard expiry, Jobs and lost-response quarantine use real APIs. Docker transport DELETE retry and the permission matrix are tested by the Docker run, not repeated as Kubernetes-specific evidence.

The exact official image digest must be available to containerd. Where cluster DNS cannot reach the registry, preload an OCI archive copied with `skopeo copy --all --preserve-digests` and verify the original index SHA-256 before importing. A legacy `docker save` archive may lose the index identity; do not relabel a different manifest with the expected digest. No custom/repacked Runner image is accepted.

**Operator-approved credential boundary:** Kubernetes Destroy refresh reads the single Runner token from the bootstrap Secret into protected plan/state/backup. The harness requires owner-only directories/files, permits that token only in the exact original Secret data, and still rejects management tokens, input/metadata/argv/env or log exposure. It records `protectedTokenCopies` explicitly. It does not claim Kubernetes tokens never enter Terraform evidence. Raw materials remain credential-grade even after the token's ephemeral registration disappears.

## Evidence and limits

Owner-only temporary directories retain config, SQLite, raw plans/state/workspaces and logs; **never upload them**. `report.json`/stdout contain bounded results and digests only. Teardown stops Shaula and deletes only fixture-owned resources; it never counts as successful Shaula reclaim. The deliberately quarantined response-loss orphan is removed only with the disposable Forgejo server and is not counted as automatic cleanup.

No real VM/cloud, real GitHub, busy-safe early idle drain, full supported-version matrix, or weighted-load distribution acceptance is claimed. Local Rust/browser regression, Terraform/runtime conformance and these real acceptance checks are separate verification layers.

## DX-30 diagnostics acceptance

`diagnostics.mjs` exercises spec 0041 against a dedicated disposable Linux VM
with its own Docker Engine. The VM may run under Hyper-V; the resource platform
under test is **Docker**, not a Hyper-V Runner backend. Use the same prerequisites
as above, plus Chromium installed through `web/node_modules/playwright/cli.js`.
The explicit opt-in is required:

```sh
node web/node_modules/playwright/cli.js install --with-deps chromium
node --test scripts/forgejo-lifecycle/faults.test.mjs
SHAULA_DX30_DISPOSABLE_VM=1 node scripts/forgejo-lifecycle/diagnostics.mjs
```

The four scenarios use the actual daemon, original bundled template, real
Forgejo registrations and real Docker resources:

| Scenario | Scoped fault and required evidence |
| --- | --- |
| no-create | Fail Docker image reads during Terraform plan; prove no container Create/Destroy request, failed plan invocation and `never_started` cleanup. |
| waiting-online | Fail only the initial Runner Declare RPC; retain the original offline registration/container, then restore and restart that same external process. |
| destroy-failure | Fail container DELETE at the explicit maximum-lifetime deadline; retain occupancy, restart and restore transport, then require both resource and registration absence before zero occupancy. Forgejo may remove an exiting ephemeral registration itself. This does not establish busy-safe ordinary drain or a separate registration-DELETE failure. |
| rollout-lag | Publish an Active revision using a second socket alias while the old Generation occupies capacity; compare old/candidate pins and observe follow after legitimate cleanup. |

For every failure checkpoint and recovery, the harness reads the real diagnostics
HTTP endpoint, SQLite domain ledger through a read-only connection, provider
inventory, and the actual embedded UI with Playwright. It does not mock browser
responses or inject ledger rows. The browser completes a real authorization-code
and PKCE flow against the disposable issuer and uses the daemon's secure session
cookie over a loopback HTTPS relay. API probes use OIDC Bearer tokens. Screenshots and bounded JSON receipts go
into the fixture's `public-evidence/` directory after credential checks; raw
SQLite, logs, plans, keys and token files remain outside that directory and must
never be published. Only a report with `passed: true` and all four checks is
DX-30 acceptance. Failed attempts and fixture teardown are not cleanup evidence.
