# Docker runner runtime policy

The reviewed Template is trusted infrastructure code executed by a Terraform
subprocess. Its host identity can access the Docker engine socket and therefore
shares the Docker host administrator trust domain. Runner containers cannot
access that socket or receive GitHub App keys, installation tokens, provider
credentials, or Shaula management credentials.

Each Generation owns one container using the unmodified official image from
`ghcr.io/actions/actions-runner`, pinned by immutable digest. Derived images,
custom image builds, injected bootstrap programs, and container-side Setup Info
fetching or processing are forbidden. The image is preloaded shared data.
The container runs as the image's non-root runner user with a 4 GiB memory
limit and CPU set `0-1`. It does not restart or auto-remove, and has no host
mounts, privileged mode, host namespaces, added devices, or added capabilities.

Terraform creates the container with `start=false` and `must_run=false`.
After apply completes, Shaula validates the returned container identity and
Generation labels, prepares the bounded sanitized `.setup_info` JSON outside
the container, copies it to `/home/runner/.setup_info`, and starts the container.
Setup Info preparation or delivery failure degrades to unavailable diagnostics;
it does not prevent the required start attempt. Startup and identity failures
remain lifecycle failures. Terraform continues to own resource creation and
destruction; the bootstrap contract permits only this fixed post-apply handoff.

The container command is `/home/runner/bin/Runner.Listener run`. Its native
`ACTIONS_RUNNER_INPUT_JITCONFIG` environment input is the only JIT transport.
The official Listener captures and removes that input from the ordinary
environment before running workflow processes. No credential appears in argv.
Docker container metadata, Terraform inputs, plans, state, and raw diagnostics
remain credential-grade until safe GitHub removal and verified Destroy.

Environment removal does not provide process-inspection isolation or memory
zeroization: workflow code in the same Runner Execution Domain may inspect
Listener memory or its initial environment. Host administrators can inspect
the Docker configuration, including the original JIT input.

Operators must preload the exact official image before executing the Template.
Changing image, bootstrap contract, bindings, engine, provider lock, or this
policy requires renewed exact-tuple validation; a smoke report is not an
activation attestation. Retained Generations keep their original artifact and
inputs for cleanup; this policy applies to newly published Template Versions.
