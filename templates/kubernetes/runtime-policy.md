# Kubernetes runner runtime policy

Each Generation owns exactly one bootstrap Secret and one Pod in an existing
publisher-selected namespace. Terraform owns their creation and destruction.
The namespace, image registry, and other cluster resources are shared data.
Only unmodified official `ghcr.io/actions/actions-runner`,
`code.forgejo.org/forgejo/runner` and `gitlab/gitlab-runner` images are
allowed, pinned by immutable digest. The publisher's `runner_backend` binding
selects GitHub (default), Forgejo or GitLab on this same Kubernetes template and
is frozen per Revision. No derived image, custom bootstrap program, init
container, sidecar, or container-side Setup Info processing is required.

All backends use restart policy `Never` and disable automatic service account
token mounting. For GitHub, the Pod runs `/home/runner/bin/Runner.Listener run`. JIT is passed only
through the official `ACTIONS_RUNNER_INPUT_JITCONFIG` input, referenced from
the bootstrap Secret. The Listener captures and removes that input from the
ordinary environment before workflow execution. It receives no Kubernetes API,
provider, GitHub App, installation-token, or Shaula management credentials.

For GitHub, the Secret starts mutable with only the `jit_config` key. The Pod requires a
Secret volume item named `.setup_info`, mounted read-only at the Runner root
using a fixed subPath. Its absence prevents container startup. The provider
accepts `Pending` as successful provisioning so apply can finish before the
bootstrap handoff; the Template never waits for an in-container helper.

After apply completes, Shaula prepares bounded sanitized Setup Info JSON
outside the Pod. It validates both returned resource UIDs and Generation
labels, then atomically adds `.setup_info` and sets `immutable=true` using
Secret UID and resourceVersion preconditions. An unavailable log projection
becomes `[]` so missing optional diagnostics do not keep the Pod blocked.
Identity or required delivery failures remain lifecycle failures and use the
normal cleanup path. The Pod starts only after Kubelet projects the completed
key. Terraform does not perform an Update or a second apply to release it.

Secret data, the original Listener environment, Terraform inputs, plans, state,
and raw diagnostics remain credential-grade. Native environment removal does
not provide process-inspection isolation or memory zeroization. API access to
Secrets remains restricted to the trusted provisioning identity; workflow
processes have no API credential or mounted JIT file.

For Forgejo, the Secret starts with only a nonsecret `runner_backend=forgejo`
marker. This keeps Kubernetes 2.33.0's plan data known without admitting unknown
bootstrap contents. A required `token` item, mounted read-only
at `/data/.forgejo-token` with a fixed subPath, gates startup. Shaula validates
resource identities and patches the token plus `immutable=true` after apply.
The official `/usr/bin/dumb-init -- /bin/forgejo-runner` command executes
`one-job --wait` with a file token URL and frozen nonsecret URL/UUID/`:host`
labels. Token bytes never enter Terraform inputs, Create plans, argv or
environment. Subsequent provider refresh/state remains credential-grade. The container
runs as UID 1000, requires non-root execution, drops ALL capabilities and
forbids privilege escalation. It receives no Docker socket or extra volume.
The token file, Secret and process memory remain credential-grade; workflow
code runs in the same Runner Execution Domain. Normal recycling does not infer
safe deletion from process exit/idle alone. The shared hard lifetime is a
separate safeguard that explicitly permits interrupting an overlong task.

For GitLab, the Secret likewise starts with only a `runner_backend=gitlab`
marker. A required `config.toml` item (one `shell` executor runner, the
registration token, nothing else), mounted read-only at
`/home/gitlab-runner/config.toml` with a fixed subPath, gates startup; Shaula
patches it plus `immutable=true` after the same identity checks. The official
`/usr/bin/dumb-init -- /usr/bin/gitlab-runner` command executes `run-single`
with that config, the frozen runner name, `--max-builds 1` and
`--wait-timeout 600`; the image's CA-update wrapper is not used. The container
runs as UID 999 (the image's packaged `gitlab-runner` user), requires non-root
execution, drops ALL capabilities and forbids privilege escalation. Its working
directory `/home/gitlab-runner` is a Generation-owned, disk-backed `emptyDir`:
`run-single` writes `.runner_system_id` beside its config and the shell
executor builds below it. The emptyDir is the only extra volume; there is no
Docker socket, host path or init container, and it is deleted with the Pod.
Token bytes never enter Terraform inputs, Create plans, argv or environment.
The `shell` executor runs jobs inside this Pod and ignores `.gitlab-ci.yml`
`image:`; the `kubernetes` executor would create Pods outside the Generation
and is not admitted. GitLab does not enforce one job per token, so Shaula
pauses the registration once a job is claimed and requires pause → recheck
before deletion. The runner log prints a 9-character token prefix and is
credential-adjacent output.

Changing the image, bootstrap contract, bindings, engine, provider lock, or this
policy requires renewed exact-tuple validation. Retained Generations keep their
original artifact, inputs, and resource evidence for cleanup. This policy
applies to newly published Template Versions.
