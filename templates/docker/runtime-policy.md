# Docker runner runtime policy

The reviewed Template is trusted infrastructure code executed by a Terraform
subprocess. Its host identity can access the Docker engine socket and therefore
shares the Docker host administrator trust domain. Runner containers cannot
access that socket or receive GitHub App keys, installation tokens, provider
credentials, or Shaula management credentials.

Administrator-owned bindings may instead select `ssh://username@host:port`.
The remote account's Docker access has the same host-admin trust implications.
SSH requires a password or a complete private key (optionally passphrase-protected),
never both. Terraform and the fixed Docker bootstrap share invocation-scoped
OpenSSH helpers under the existing process-tree fence. Credentials are not
placed in argv or environment values and never enter a container; temporary
files are private (0700 directory / 0600 files on Unix) and removed on ordinary
completion, failure or cancellation. Abrupt host/process death can leave private
temporary files; the service temporary directory remains credential-grade.
Destroy reconstructs authentication from the original protected input, not an
expired temporary path. The immutable Revision, inputs, plan and state remain
credential-grade. OpenSSH must be installed on the Shaula host and Docker CLI
must be available without sudo on the SSH target.

Host key verification is mandatory. Operators supply verified `ssh_known_hosts`
entries or provision the service user's known_hosts before use. Unknown or
changed keys fail closed. Ambient SSH config, agent identities/forwarding and
connection multiplexing are disabled. The fixed helper supplies no arbitrary
SSH options or remote command bindings; only Docker's dial-stdio is used.
Plaintext TCP endpoints remain forbidden. Direct Terraform invocation does not
prepare these helpers and is not the supported SSH execution path.

Each Generation owns one container using the unmodified official image from
`ghcr.io/actions/actions-runner` or `code.forgejo.org/forgejo/runner`, pinned by
immutable digest. The publisher's `runner_backend` binding selects GitHub
(default) or Forgejo on this same Docker template and is frozen per Revision. Derived images,
custom image builds, injected bootstrap programs, and container-side Setup Info
fetching or processing are forbidden. The image is preloaded shared data.
The container runs as the image's non-root runner user with a 4 GiB memory
limit and CPU set `0-1`. It does not restart or auto-remove, and has no host
mounts, privileged mode, host namespaces, added devices, or added capabilities.

Terraform creates the container with `start=false` and `must_run=false`.
For GitHub, after apply completes, Shaula validates the returned container identity and
Generation labels, prepares the bounded sanitized `.setup_info` JSON outside
the container, copies it to `/home/runner/.setup_info`, and starts the container.
Setup Info preparation or delivery failure degrades to unavailable diagnostics;
it does not prevent the required start attempt. Startup and identity failures
remain lifecycle failures. Terraform continues to own resource creation and
destruction; the bootstrap contract permits only this fixed post-apply handoff.

The GitHub container command is `/home/runner/bin/Runner.Listener run`. Its native
`ACTIONS_RUNNER_INPUT_JITCONFIG` environment input is the only JIT transport.
The official Listener captures and removes that input from the ordinary
environment before running workflow processes. No credential appears in argv.
Docker container metadata, Terraform inputs, plans, state, and raw diagnostics
remain credential-grade until safe GitHub removal and verified Destroy.

Environment removal does not provide process-inspection isolation or memory
zeroization: workflow code in the same Runner Execution Domain may inspect
Listener memory or its initial environment. Host administrators can inspect
the Docker configuration, including the original JIT input.

For Forgejo, Shaula verifies the stopped container and its anonymous `/data`
volume, copies the protected token to `/data/.forgejo-token`, and starts the
container. The official `dumb-init` entrypoint executes `/bin/forgejo-runner
one-job` with a `file://` token URL, the frozen instance URL/UUID/`:host` labels,
and `--wait`. A fixed nonsecret `SHAULA_RUNNER_BACKEND=forgejo` environment
marker keeps Docker's planned environment known for strict admission; unknown
values are not accepted. Token bytes never enter Terraform, argv or environment. The token
file and process memory remain inside the trusted Runner Execution Domain;
this is not isolation from workflow code. No Docker socket or DinD is allowed.
The container never restarts; Terraform Destroy removes its anonymous volume
(`remove_volumes=true`). Exit/idle alone does not prove a task was unassigned.
Normal recycling retains the backend's ownership and safe-removal gates; the
shared hard lifetime is a separate, explicitly task-interrupting safeguard.

Operators must preload the selected backend's exact official image before executing the Template.
Changing image, bootstrap contract, bindings, engine, provider lock, or this
policy requires renewed exact-tuple validation; a smoke report is not an
activation attestation. Retained Generations keep their original artifact and
inputs for cleanup; this policy applies to newly published Template Versions.
