# AWS runner runtime policy

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
