# Official Docker bootstrap probe, 2026-09-09

This bounded platform probe used the new `templates/docker` source with Terraform
1.9.8, Docker CLI 29.6.2, Docker provider 3.0.2 and the pinned official Runner
2.337.0 image. It ran
in an isolated temporary workspace on the Docker host, without modifying the
production Shaula service or an active Fleet. Generation:
`141b0d2e-f1bc-45f3-bf06-c47ec68e72dd`.

Observed results:

- One Terraform Create apply produced a container in `created` state, with the
  official Listener not started.
- A host-created `.setup_info` was copied into that stopped container and read
  back. A root-owned mode `0444` file was readable by the image's UID 1001.
- Starting the container invoked the official Listener. It rejected deliberately
  invalid synthetic JIT and exited; no real GitHub workflow job was executed.
- Terraform delete-only Destroy completed and `terraform state list` was empty.

Both Docker and Kubernetes templates separately passed Terraform 1.9.8 format,
readonly locked init and validate in isolated Linux copies. Kubernetes uses a
real HashiCorp 2.33.0 provider lock; these checks did not create Kubernetes objects.
The saved-plan admission tests also accepted both actual provider plan shapes:
the Docker probe's plan and a Kubernetes plan produced against an isolated mock
Namespace endpoint. Only synthetic JIT and metadata were used in these probes.

This evidence establishes stopped-container copy/start behavior, official
Listener invocation, file readability and this probe's cleanup. It does not
establish production Runtime/archive integration, valid JIT consumption, first-job
Set up job display, Kubernetes runtime behavior, crash recovery or full conformance.
No credentials, raw Terraform input/state, platform response or remote source copy
is included in this evidence record.
