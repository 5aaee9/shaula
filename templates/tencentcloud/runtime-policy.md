# Tencent Cloud runner runtime policy

`bindings.runner_backend` selects `github` (default) or `forgejo` for this
immutable Template Revision, never a Fleet parameter. JIT/Listener details
below describe GitHub; the Forgejo cloud-init contract is specified separately.

Each Generation owns exactly one pay-as-you-go CVM instance. The pinned
`tencentcloudstack/tencentcloud` provider owns creation and destruction.
The publisher binds an existing Ubuntu Server 22.04 LTS x86_64 image with
cloud-init and systemd, a VPC, matching subnet/zone and security groups.
These shared resources are not managed or deleted. The explicit
`shaula.tencentcloud-image/v1` contract trusts the publisher's image selection;
an image ID is not a content digest and no immutable OCI image is claimed.
Availability, quotas, image suitability and permissions need target validation.

The instance uses POSTPAID_BY_HOUR billing and an encrypted CLOUD_SSD system
disk released with the instance. Termination protection is disabled. No data
disk, SSH key, password, CAM role, EIP, network or inbound rule is created.
Public bandwidth defaults to zero (no public IP), requiring publisher-provided
NAT egress. Positive bandwidth allocates an instance public IP with
TRAFFIC_POSTPAID_BY_HOUR billing. The selected zone/type must support the disk,
and the account must authorize encryption through its KMS service.

Cloud-init writes a root-owned 0600 JIT file, a preparation file, the fixed
bootstrap and a systemd unit. User-data is base64 encoded exactly once for
the provider; nested sensitive file content is encoded as data, never evaluated
as shell or Terraform code. Encoded user-data is limited to 16 KiB, including
the JIT and optional preparation. Preparation is trusted publisher Bash code,
receives no JIT environment, must not contain credentials, and failure prevents
installation and registration. There are no Fleet parameters.

Bootstrap installs download tools and the pinned GitHub Actions runner
2.337.0 tarball, verifies its artifact-recorded SHA-256 before extraction,
installs its runtime dependencies and runs the Listener as unprivileged user
`runner`. Ubuntu repositories, GitHub release assets and Actions endpoints
must be reachable. Server-driven runner updates may change the executed
version. Bootstrap runs once after cloud-final; the unit is never enabled,
has Restart=no and starts in an existing directory. A durable marker prevents
reuse on reboot or after partial failure. Bootstrap restricts local cloud-init
caches, reads and deletes the JIT file, then passes the official
ACTIONS_RUNNER_INPUT_JITCONFIG environment input directly to Runner.Listener.
There is no host-side Setup Info delivery for this VM contract.

CAM credentials are provider-only, never guest data, tags or runner environment.
Use a least-privilege CAM identity scoped to CVM lifecycle, image/network
describe, tags and encrypted disk operations. User-data and metadata remain
credential-grade for the instance lifetime. This template does not enforce
CVM metadata token authentication; guest processes with metadata access or
root privileges may retrieve JIT. Cloud console output, Terraform inputs,
plans, state and raw diagnostics also require privileged protection.

Destruction follows Shaula's existing GitHub safe-removal gate and uses the
frozen Terraform state/material, not a tag-based cleanup. Unknown outcomes
retain state for recovery; running or stopped CVM status is not runner idleness.
Static validation and mock plans do not prove real boot, registration, jobs,
interruption recovery or instance/system-disk cleanup. Accept those separately
on the target Tencent Cloud environment before production use.

## Forgejo cloud-init contract

`shaula.forgejo-vm-cloud-init/v1` explicitly permits `shaula.forgejo_vm.token`
in protected Terraform input and cloud-init. Shaula pre-registers exactly one
ephemeral Runner; only its token and UUID/URL/labels reach the VM, never Forgejo
management/CAM credentials. The official native `forgejo-runner-13.1.0-linux-amd64`
release is fixed to SHA-256
`29dae21e93f0eab5cdf3564008d44603c74770b41a4f4f1aceed172c774bc376`.
There is no guest registration, custom Runner build or latest resolution.
Ubuntu packages, Forgejo release assets and the selected instance need egress.

Bootstrap claims the durable one-use marker before preparation, verifies the
binary, restricts cloud-init caches, and copies the token to the runner-owned
0700 `/run/shaula-forgejo` directory as a 0600 file, removing the initial handoff.
JSON identity is data, never shell code. The service has Restart=no,
NoNewPrivileges=true and no boot enablement, dropping to a non-root runner.
`one-job --wait` reads `--token-url file:...`, not argv/environment credentials.
Only explicit `:host` labels are admitted (this disposable VM); tools beyond git,
such as Node, need publisher provisioning. Encoded user-data including the token
must still fit the 16 KiB bound. No Setup Info delivery is added.

Forgejo enforces at most one job but does not delete the CVM. Shaula destroys
original owned resources after exact registration absence, or the shared hard
lifetime (default 7200 seconds after successful Create, even Busy). Idle alone
is not safe-drain proof. Failures never restart this seed. Terraform input,
plans/state and CVM user-data/metadata can retain the per-Runner token; sensitive
flags/base64 are not encryption or guest isolation. Real Forgejo job and CVM/disk
cleanup acceptance is still required on the target deployment.
