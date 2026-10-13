# Alibaba Cloud runner runtime policy

`bindings.runner_backend` selects `github` (default), `forgejo` or `gitlab` for
this immutable Template Revision, never a Fleet parameter. JIT/Listener details
below describe GitHub; the Forgejo and GitLab cloud-init contracts follow.

Each Generation owns exactly one pay-as-you-go ECS instance. The pinned
`aliyun/alicloud` provider owns creation and destruction. The publisher binds
an existing Ubuntu Server 22.04 LTS x86_64 image with systemd and cloud-init
supporting Alibaba Cloud IMDSv2, a vSwitch and security groups in its VPC.
These shared resources are not managed or deleted. The explicit
`shaula.alicloud-image/v1` contract trusts the publisher's image selection;
an image ID is not a content digest and no immutable OCI image is claimed.
Availability, quotas, image suitability and permissions need target validation.

The instance uses PostPaid billing without spot interruption and an encrypted
cloud_essd system disk released with the instance. Deletion protection is
disabled. No data disk, SSH key, password, RAM role, EIP, network or inbound
rule is created. Public bandwidth defaults to zero (no public IP), requiring
publisher-provided NAT egress. Positive bandwidth allocates an instance public
IP with PayByTraffic billing. The vSwitch zone/type must support the disk,
and the account must authorize encryption through its KMS service. IMDSv2 is
required; this restricts metadata requests but does not hide JIT from jobs.

Cloud-init writes a root-owned 0600 JIT file, a preparation file, the fixed
bootstrap and a systemd unit. User-data is base64 encoded exactly once for
the provider; nested sensitive file content is encoded as data, never evaluated
as shell or Terraform code. The template conservatively limits encoded
user-data to 16 KiB, keeping decoded data below ECS's limit as well, including
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

RAM credentials are provider-only, never guest data, tags or runner environment.
Use a least-privilege RAM identity scoped to ECS lifecycle, image/network
describe, tags and encrypted disk operations. User-data and metadata remain
credential-grade for the instance lifetime. Guest processes with metadata
access or root privileges may retrieve JIT. Cloud console output, Terraform
inputs, plans, state and raw diagnostics also require privileged protection.

Destruction follows Shaula's existing GitHub safe-removal gate and uses the
frozen Terraform state/material, not a tag-based cleanup. Unknown outcomes
retain state for recovery; running or stopped ECS status is not runner idleness.
Static validation and mock plans do not prove real boot, registration, jobs,
interruption recovery or instance/system-disk cleanup. Accept those separately
on the target Alibaba Cloud environment before production use.

## Forgejo cloud-init contract

`shaula.forgejo-vm-cloud-init/v1` explicitly permits `shaula.forgejo_vm.token`
in protected Terraform input and cloud-init. Shaula pre-registers exactly one
ephemeral Runner; only its token and UUID/URL/labels reach the VM, never Forgejo
management/RAM credentials. The official native `forgejo-runner-13.1.0-linux-amd64`
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

Forgejo enforces at most one job but does not delete the ECS instance. Shaula
destroys original owned resources after exact registration absence, or the shared
hard lifetime (default 7200 seconds after successful Create, even Busy). Idle
alone is not safe-drain proof. Failures never restart this seed. Terraform input,
plans/state and ECS user-data/IMDS can retain the per-Runner token; sensitive
flags/base64 and IMDSv2 are not encryption or job isolation. Real Forgejo job and
ECS/disk cleanup acceptance is still required on the target deployment.

## GitLab cloud-init contract

`shaula.gitlab-vm-cloud-init/v1` explicitly permits `shaula.gitlab_vm.config_toml`
in protected Terraform input and ECS user-data. Shaula first registers exactly one
GitLab Runner, then renders its single-registration token into that canonical
`config.toml` (URL, Runner id/name, token, `executor = "shell"`). Any other
executor, extra key or non-`glrt-` token is rejected; the GitLab Auth Profile
token and RAM credentials never enter the guest. The guest needs Ubuntu
x86_64 or arm64 with cloud-init/systemd and util-linux. Bootstrap installs the
official `gitlab-runner-linux-<arch>` 19.4.1 binary from
`gitlab-runner-downloads.s3.amazonaws.com`, pinned to SHA-256
`5a159b36c5ff54fda9248e00bb0cdfb62c1112ab3741f5282c04a41e94da1306` (amd64) or
`9763a63f7bc2319119f80de41d1d414465b5268d072bcd55266020b989e93ee9` (arm64).
There is no guest `register`, `.deb` package service, custom build or latest lookup.

The root bootstrap claims the one-use marker before preparation, protects cloud-init caches, copies
the handoff to `/run/shaula-gitlab/config.toml` (0600 inside a runner-owned 0700
directory) and removes it. It drops to the non-root `runner` user and runs
`gitlab-runner run-single --config <file> --runner <name> --max-builds 1
--wait-timeout 600`; the token is neither argv nor environment. The service has
Restart=no, NoNewPrivileges=true and no boot enablement, because a restarted
process would claim a second job with the same token. The shell executor runs
the job directly on this disposable VM as `runner` and ignores job `image:` and
`services:`. Runner journal output contains the short token and is
credential-adjacent. Ubuntu packages, the GitLab download endpoint and the
selected instance must be reachable.

`--max-builds 1` ends the process after one job; it does not delete the
ECS instance. Shaula pauses the registration, verifies it idle, deletes it and then
destroys owned resources, or applies the shared hard lifetime (default 7200
seconds after Create succeeds, even Busy). Failed bootstrap never replays this
seed. Terraform inputs/plans/state and ECS user-data/IMDS retain the Runner token;
base64 and sensitive flags are not encryption or job isolation. Encoded user-data
including config.toml must fit the 16 KiB bound. No Setup Info is added.
Real GitLab job and cleanup acceptance remains deployment-specific.
