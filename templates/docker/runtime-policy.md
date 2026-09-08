# Docker runner runtime policy

The reviewed Template is trusted infrastructure code executed by a Terraform
subprocess. Its host identity can access the Docker engine socket and therefore
shares the Docker host administrator trust domain. Runner containers cannot
access that socket or receive GitHub App keys, installation tokens, provider
credentials, or Shaula management credentials.

Each Generation owns one container, with a fixed image digest and bounded
ownership labels. The image is preloaded shared data. The container runs as
the image's non-root runner user with a 4 GiB memory limit and CPU set `0-1`.
It does not restart or auto-remove, and has no host mounts, privileged mode,
host namespaces, added devices, or added capabilities.

The provider uploads a root-owned 0644 JIT file into the image's runner-owned
0700 `/shaula` directory. The pinned shim reads the file once, removes it before
launching Runner.Listener, and passes JIT only through
`ACTIONS_RUNNER_INPUT_JITCONFIG`. The Listener captures and unsets that variable
before running ordinary workflow processes. Invalid or missing JIT fails closed.

This is not process-inspection isolation or memory zeroization: workflow code
in the same Runner Execution Domain may inspect Listener memory or initial
environment. Terraform inputs, plans, state, and raw diagnostics remain
credential-grade files through safe GitHub removal and verified Destroy.

The local registry address in the image alias records the private image import.
It is not a registry service dependency at runtime: operators must preload the
exact image before executing the Template. Docker image pruning removes that
prerequisite. Changing image, bindings, engine, provider lock, or this policy
requires renewed exact-tuple validation; a smoke report is not an activation
attestation.
