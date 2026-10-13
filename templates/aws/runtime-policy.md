# AWS runner runtime policy

`bindings.runner_backend` selects `github` (default), `forgejo` or `gitlab` for
this immutable Template Revision, never a Fleet parameter. JIT/Listener details
below describe GitHub; the Forgejo and GitLab cloud-init contracts follow.

Each Generation owns exactly one EC2 instance. Terraform provider
`hashicorp/aws` (locked version in `.terraform.lock.hcl`) owns its entire
resource lifecycle. The provider resolves the Canonical Ubuntu AMI through an
`aws_ami` data source pinned to owner account `099720109477` with
architecture `x86_64`, virtualization `hvm`, root-device `ebs` and state
`available`; the bound name pattern selects the publication channel and
`most_recent` resolves the newest match at plan time. A dated serial suffix in
`aws_ami_name` yields an immutable image; the wildcard default tracks
Canonical's latest Jammy publication. The instance launches once with
cloud-init user-data; host-boot autostart and termination protection are
disabled, the root volume is encrypted gp3 and deleted on termination, and
IMDSv2 is required with hop limit one. Destruction terminates the owned
instance; the AMI is a data source, not managed state, and its later
unavailability does not block Destroy.

The guest image is Canonical's official Ubuntu publication and explicitly
uses `shaula.aws-ami/v1`, without claiming a container image digest or a
frozen AMI ID. The runner software is artifact-pinned material: bootstrap
downloads `actions-runner-linux-x64-<version>.tar.gz` from GitHub releases,
verifies the sha256 recorded in this template, extracts to
`/opt/actions-runner` and creates an unprivileged `runner` user. There is no
`latest` resolution; the Listener's server-driven self-update still governs
the version that ultimately executes a job. AMI selection, IAM permissions
of the bound credential and network egress remain the publisher's trust
responsibility.

Cloud-init writes a mandatory root-owned bootstrap and a systemd service
with `Restart=no`. The service waits for cloud-final, is started once
without blocking cloud-init, and is never enabled at boot. A durable startup
marker prevents re-registration after a manual restart or interrupted
bootstrap. An optional publisher-authored Bash preparation body runs first,
without a JIT environment or parameter substitution; it is privileged code,
must not contain credentials, and cannot replace the mandatory install or
bootstrap contract.

JIT is base64-encoded as data in a root-owned `0600` file, never
interpolated into shell code or command arguments. Before starting the
Listener, bootstrap restricts guest cloud-init caches to root, reads and
deletes the JIT handoff file, and drops to the `runner` user. It runs
`/opt/actions-runner/bin/Runner.Listener run` with the official
`ACTIONS_RUNNER_INPUT_JITCONFIG` input. The Listener captures and removes
the input before running the ephemeral job. It receives no AWS credential,
GitHub App, installation-token, or Shaula control-plane credentials. This
contract does not deliver the container-only completed-apply Setup Info.

EC2 user-data remains retrievable through IMDSv2 by any guest process able
to reach the metadata endpoint until the instance terminates; it contains
the JIT and therefore stays credential-grade for the instance lifetime.
Terraform inputs/plans/state, provider temporary files, EC2 console output
and raw diagnostics are credential-grade. Guest permissions do not isolate a
job that can become root or read instance metadata. The instance's subnet,
security groups and outbound reachability are publisher-reviewed bindings;
the template attaches the VPC default security group when none is bound and
creates no inbound rules itself.

The bound IAM credential needs only EC2 instance/volume and AMI describe
operations; it is never delivered to the instance, which carries no IAM
instance profile. Launch failures such as insufficient capacity, throttling
or an unavailable default subnet are platform errors surfaced through the
apply log, not Shaula cancellations. Real instance boot, cloud-init, runner
download and verification, JIT registration, job execution and cleanup must
be accepted on the target AWS environment separately.

## Forgejo cloud-init contract

`shaula.forgejo-vm-cloud-init/v1` explicitly permits `shaula.forgejo_vm.token`
in protected Terraform input and cloud-init. Shaula first registers exactly
one ephemeral Runner, then supplies its UUID/URL/labels and per-Runner token;
no Forgejo management token or cloud-provider credential enters the guest.
The pinned official `forgejo-runner-13.1.0-linux-amd64` binary is downloaded
from the Forgejo release and verified with artifact-pinned SHA-256
`29dae21e93f0eab5cdf3564008d44603c74770b41a4f4f1aceed172c774bc376`.
There is no guest registration command, custom Runner build or latest lookup.

The root-owned bootstrap claims the one-use marker before preparation, protects
cloud-init caches, copies the private token into `/run/shaula-forgejo/token`
(0600 inside a runner-owned 0700 directory), and removes the initial handoff.
The service is not enabled on reboot, has Restart=no and NoNewPrivileges=true;
it drops to a non-root runner user. JSON identity is read as data, never shell
syntax. The fixed command is `one-job --wait --token-url file:...`; the token
is not argv/environment. Only explicit `:host` labels are admitted, meaning
this disposable VM. Workflow tools beyond git (such as Node) require reviewed
image/preparation provisioning. Ubuntu packages, the Forgejo release endpoint
and the selected instance must be reachable.

Ephemeral is enforced by Forgejo: at most one job, not automatic EC2 deletion.
Shaula destroys owned resources after exact registration absence, or at the
shared hard lifetime (default 7200 seconds after Create succeeds, even Busy).
A live idle registration alone is not safe-drain proof. Failed bootstrap never
replays this seed; retained registration/resources are reconciled conservatively.
Terraform inputs/plans/state and EC2 user-data/IMDS still contain the Runner
token; base64 and sensitive flags are not encryption or job isolation. Only
this disposable Runner token may cross that boundary. No Setup Info delivery
is added. Real Forgejo job/VM cleanup acceptance remains deployment-specific.

## GitLab cloud-init contract

`shaula.gitlab-vm-cloud-init/v1` explicitly permits `shaula.gitlab_vm.config_toml`
in protected Terraform input and EC2 user-data. Shaula first registers exactly one
GitLab Runner, then renders its single-registration token into that canonical
`config.toml` (URL, Runner id/name, token, `executor = "shell"`). Any other
executor, extra key or non-`glrt-` token is rejected; the GitLab Auth Profile
token and AWS credentials never enter the guest. The guest needs Ubuntu
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
EC2 instance. Shaula pauses the registration, verifies it idle, deletes it and then
destroys owned resources, or applies the shared hard lifetime (default 7200
seconds after Create succeeds, even Busy). Failed bootstrap never replays this
seed. Terraform inputs/plans/state and EC2 user-data/IMDS retain the Runner token;
base64 and sensitive flags are not encryption or job isolation. No Setup Info
is added. Real GitLab job and cleanup acceptance remains deployment-specific.
