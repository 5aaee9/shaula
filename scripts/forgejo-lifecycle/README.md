# Forgejo lifecycle acceptance

This harness starts **actual `shaula serve`**, isolated SQLite and a disposable HTTPS OIDC issuer. It publishes the bundled artifact and credential through authenticated HTTP; the production supervisor and Terraform own Runner Create/Destroy. No fake runtime or direct Runner start/registration substitutes for the lifecycle. Permission-only probes separately create/delete registrations to test scoped API authorization.

Prerequisites: Linux with writable cgroup v2 delegation and `cgroup.kill`, real Docker Engine (not Podman), Node 22.13+, OpenSSL, tar, Terraform **1.9.8**, and a built Shaula binary. The test parent must run inside the delegation; `scripts/lifecycle-acceptance.sh` derives `SHAULA_TEST_CGROUP` from that parent. The caller must be in the Docker host network namespace (enter the RootlessKit child user/mount/network namespace for rootless Docker). Only generated test workflows run. Do not expose fixture ports on a production host.

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
- Read authoritative HTTP state and retained inputs from owner-only SQLite in read-only mode. Require atomic sealed state and a completion receipt after reclamation; allow receipt-authorized workspace reaping and reject normal local/backup/emergency state files. Sealing prohibits writes while preserving bounded completion replay; it is distinct from claim revocation.

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

## Worker crash, restoration and pressure

The Docker run additionally executes an offline full-data-set backup/restore,
small-host Worker saturation and an accepted Docker Create whose response is
withheld across daemon SIGKILL. Reports distinguish quiescent restoration from
post-checkpoint divergence: the rehearsal creates a real later resource and
requires the restore procedure to refuse replacing any newer authoritative file,
then resumes that newer set to reclaim the exact resource. This guarded rehearsal
is not an automatic production disaster-recovery command. Waiting-worker
measurements are scoped to a four-worker configuration, not maximum-count
capacity. An interrupted Create must retain its actual external
resource and occupancy in quarantine; fixture teardown is not recovery evidence.

On an explicitly disposable Linux host, `SHAULA_ACCEPTANCE_CONTROL_FAULTS=1`
also enables a root loopback transport proxy. The fixture must run as a non-root
user with noninteractive sudo. One iptables OUTPUT rule matches only that UID,
one private loopback destination port and this fixture's random comment; teardown
removes that exact rule. No public listener or production authentication change
is involved. The root proxy forwards authenticated bytes without logging them.
It tests lost committed Spawn ACK responses, a crash after exec but before Create,
DELETE committed before a delayed Create-start request, and a crash after the
Create intent commits but before its response reaches the worker.
It also makes every state method unavailable after a real Docker Create,
requires retained emergency state across restart, and replays exact authenticated
log requests under a 32-request in-flight bound during the capacity scenario.
State must continue within that scenario's five-second response budget. Duplicate
log requests preserve their original IDs and bytes and do not fabricate events.
Failure teardown fences only the recorded child cgroups of this fixture before
removing its disposable external resources; that teardown is never counted as
normal lifecycle cleanup.

Hard-expiry Docker deletion and CI-registration retry use separate checkpoints.
Docker provider 3.0.2 stops a container before issuing DELETE, so a failed removal
cannot imply that its former Busy runner is still running. Registration retry
is checked with an unassigned idle runner instead of relying on a completed
ephemeral task's registration remaining present.

## Real GitHub lifecycle with Browser dispatch

`github-run.mjs` starts a separate data directory and daemon with the actual
binary, pinned Terraform, the Docker template and an existing GitHub App profile.
Run it only on an operator-approved host already entrusted with that App key.
The input is an owner-only JSON file containing the normal v2 auth-profile PUT
payload. Never copy the key into workflow secrets, command arguments or reports.
The script narrows the test to `5aaee9/shaula` and uses a fresh random Scale Set.

```sh
export SHAULA_ACCEPTANCE_GITHUB_PROFILE=/private/existing-test-profile.json
export SHAULA_TEST_CGROUP=/sys/fs/cgroup/your-delegated-test-service
export SHAULA_BIN=/absolute/path/to/shaula
export TERRAFORM_BIN=/absolute/path/to/terraform
node scripts/forgejo-lifecycle/github-run.mjs
```

Run the parent within the specified delegation. After it prints
`await-browser-dispatch`, use Browser to run **Shaula Docker smoke** on the exact
PR branch and enter the emitted `runner_label`. The job remains Busy for 45
seconds so the harness can restart the real daemon and require cleanup-only
recovery. Verify the same workflow's conclusion and final GitHub runner inventory
in Browser; a local `passed` report alone does not establish that external check.
The report pins the executed binary/source/runtime, requires one Generation,
retained GitHub Jobs, zero occupancy and absent container/credential volumes,
then retires the test Fleet. On failure it retains evidence and external
resources for reconciliation instead of force-removing an uncertain runner.

These scripts require completed, reviewed run evidence before their scenarios
can be marked accepted; their presence is not a full LW-11–15/28/30/32 pass.

CI runs baseline Docker/Kubernetes and the `backup`, `pressure`, `control`,
`backend-outage`, and `interrupted-create` suites in separate disposable jobs.
`SHAULA_ACCEPTANCE_SUITE` selects one suite (`all` remains the local default).
This lets a failed checkpoint retain its own evidence without suppressing the
remaining matrix. The baseline jobs also run the complete Linux process tests.

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
