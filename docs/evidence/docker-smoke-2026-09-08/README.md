# Real Docker Template smoke, 2026-09-08

One runner was created on molecule by executing the bundled Terraform Docker
Template as the same `shaula` OS user with its explicitly authorized `docker`
supplementary group. This was an external subprocess smoke, not a Fleet,
`shaula job`, exec Driver, or activation conformance run.

## Observed lifecycle

1. Terraform 1.9.8 initialized the real provider lock, validated the template,
   and admitted a saved plan containing exactly one container creation.
2. The existing GitHub App's personal-account installation issued the JIT
   configuration for `5aaee9/shaula`; runner ID 2 became online and idle.
3. Docker inspection confirmed the fixed image, non-root user, no privileged
   settings, no host mounts/socket, no restart/auto-remove, no JIT in declarative
   metadata or process argv, and removal of `/shaula/jit_config`.
4. [Smoke run 34215174154](https://github.com/5aaee9/shaula/actions/runs/34215174154)
   succeeded on runner ID 2, `shaula-codex-docker-smoke-20260908-01`. The job also
   verified Linux, the Docker marker, and absence of JIT in its environment.
5. Authenticated GitHub inventory confirmed the ephemeral registration absent.
   Terraform then applied an exact delete-only saved plan using original inputs
   and state; both empty state and the container's absence were verified.

[lifecycle.json](lifecycle.json) is the sanitized external report. Its SHA-256
is `3d15f3aabfd41aea3cf087e6918d8e4e78a6ac5f1e8e3656851f9371da41f1a2`.
[github-job.json](github-job.json) supplies the separate GitHub identity/job
evidence referenced by that report. No state, plan, JIT, private key, token, or
raw provider/runner log is included here.

## Exact tested runtime

- Docker/Moby 29.6.2, Linux amd64, API 1.55 (minimum 1.40).
- Terraform 1.9.8, unchanged vendor binary; provider `kreuzwerker/docker` 3.0.2.
- Runner 2.337.0 with the reviewed Python bootstrap shim.
- Final imported image digest:
  `sha256:eb9fa6d0a3b8688f0f1daba9989f9323ad02b02c6d1cd36d49c2fddcb7fb6201`.
- The image was transferred privately over SSH using a loopback-only temporary
  registry, then preloaded into Docker. That registry was removed after import.
  The `localhost:5001` alias is a local image identity, not a running dependency.
- Template, input, lock, provider and engine commitments are recorded in the
  report. The protected report/source ledger remains on the execution host.

## Boundaries and findings

The report deliberately retains `full_conformance_passed: false`. It proves
the real template/container/GitHub job lifecycle, not the daemon's independent
worker, HTTP state backend, crash recovery, or all management/telemetry redaction
contracts. No Template was activated and no attestation was fabricated.

Live execution exposed details absent from mocks: the data source exports `id`,
not `image_id`; GitHub JIT uses string `AgentId` and PascalCase fields; NixOS Moby
may have an empty platform brand; `docker top` requires a PID column; and Terraform
Destroy leaves resource preconditions `unknown` because it does not evaluate
them. The smoke only accepts that last case for an exact bound deletion with
no problems; arbitrary unknown checks still fail closed.

The test container and its GitHub registration were removed. An additional
unstarted registration was also removed; no standby runner is left online.
Docker/NAT and the explicitly authorized Shaula socket group remain enabled.
