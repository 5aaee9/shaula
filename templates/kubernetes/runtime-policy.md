# Kubernetes runner runtime policy

Each Generation owns exactly one bootstrap Secret and one Pod in an existing
publisher-selected namespace. Terraform owns their creation and destruction.
The namespace, image registry, and other cluster resources are shared data.
Only the unmodified official `ghcr.io/actions/actions-runner` image is allowed,
pinned by immutable digest. No derived image, custom bootstrap program, init
container, sidecar, or container-side Setup Info processing is required.

The Pod runs `/home/runner/bin/Runner.Listener run` with restart policy `Never`
and automatic service account token mounting disabled. JIT is passed only
through the official `ACTIONS_RUNNER_INPUT_JITCONFIG` input, referenced from
the bootstrap Secret. The Listener captures and removes that input from the
ordinary environment before workflow execution. It receives no Kubernetes API,
provider, GitHub App, installation-token, or Shaula management credentials.

The Secret starts mutable with only the `jit_config` key. The Pod requires a
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

Changing the image, bootstrap contract, bindings, engine, provider lock, or this
policy requires renewed exact-tuple validation. Retained Generations keep their
original artifact, inputs, and resource evidence for cleanup. This policy
applies to newly published Template Versions.
