# Forgejo lifecycle acceptance

Unlike `scripts/forgejo-e2e.sh`, this harness starts the **actual `shaula serve` binary** with an isolated SQLite database and a disposable HTTPS OIDC issuer. It publishes the bundled Docker artifact and Forgejo credential through authenticated HTTP, then lets the production supervisor and Terraform runtime own Runner Create/Destroy. No fake runtime, direct runner registration, or direct runner start is used.

Prerequisites: Linux, real Docker Engine (Podman is rejected), Node 22+, OpenSSL, tar, the pinned Terraform 1.9.8 executable, and a built Shaula binary. The caller must be in the Docker host's network namespace; for a rootless Engine enter its RootlessKit child user/mount/network namespace. Do not expose the fixture ports outside a disposable development/test host. Only generated test workflows are executed.

```sh
npm ci --prefix web
npm run build --prefix web
cargo build --locked -p shaula
export DOCKER_HOST=unix:///var/run/docker.sock
export TERRAFORM_BIN=/absolute/path/to/terraform
# Optional: DOCKER_BIN and SHAULA_BIN are exact executable paths.
node --test scripts/forgejo-lifecycle/faults.test.mjs
node scripts/forgejo-lifecycle/run.mjs
```

Docker 29 can require an explicit minimum API compatibility setting for the bundled Docker provider 3.0.2. Configure this only on the disposable Engine, not by altering a production daemon. The harness uses the existing template without modifying its command, network, image, or runtime contract.

Scenarios:

- Queued demand creates one ephemeral runner; success and failure both end in exact registration absence, Generation Destroyed, zero occupancy, container and anonymous credential volume absence.
- Real Jobs API: retain the running Forgejo job with no synthetic Scale Set ID or verified Runner association; after disappearance retain Unknown, never infer the workflow result.
- Kill/restart `shaula serve` during a running job: retain the one Generation, complete the job, and reclaim it.
- Temporarily configure `runner.max_lifetime_secs: 15`: reclaim a waiting runner, and interrupt/reclaim an active runner. These are hard-deadline tests, **not idle-safe drain evidence**.
- Test-only transports fail Docker container DELETE, then Forgejo registration DELETE. Restart the daemon at each checkpoint; occupancy stays held until both sides converge. Task acquisition traffic is forwarded unchanged; this is not a production acquisition proxy.

The owner-only temporary directory retains config, SQLite, Terraform state/workspaces and daemon logs. These are credential-grade and must not be uploaded. `report.json` and stdout contain only bounded scenario results and commitments; they are not a full template conformance attestation. On failure teardown removes only this fixture's disposable resources after stopping Shaula. Teardown never counts as a successful lifecycle assertion. A failed teardown is reported as failure, not ignored.

Not covered: cloud VM/Kubernetes runtime acceptance, minimum scoped Forgejo permissions, safe early drain of a waiting runner, lost registration responses, or real GitHub regression. The shared local regression suite remains required independently.
