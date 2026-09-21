# Proxmox runner runtime policy

`bindings.runner_backend` selects `github` (default) or `forgejo` for this
immutable Template Revision, never a Fleet parameter. JIT/Listener details
below describe GitHub; the Forgejo cloud-init contract is specified separately.

Each Generation owns exactly one cloned QEMU VM and one NoCloud seed ISO.
Terraform provider `indexyz/proxmox` 0.5.1 owns their entire resource lifecycle.
The provider finds exactly one visible template by its case-sensitive name,
clones on that template's node, allocates a free VMID at or above the bound,
attaches the seed to `ide2`, and starts the VM once. The clone is a linked
clone by default; the publisher may request a full clone, and a linked clone
requires template disks on storage Proxmox supports for them. The VM inherits
root disks and its one Ethernet NIC; Fleet-approved `cpu_cores` and `memory_mb`
parameters set its virtual CPU count and memory in MiB. Its seed requests IPv4 DHCP for
an `eth*` or `en*` interface; networking is not a publisher or Fleet option.
Host-boot autostart and deletion protection are disabled. Refresh does not
restart a stopped guest. Destruction stops and removes the owned VM before
deleting its exact seed volume; stop failure retains state for recovery.

The VM image is operator-owned and explicitly uses
`shaula.proxmox-template/v1`, without claiming a container image digest. The
publisher must maintain a clean Linux template with systemd, cloud-init,
util-linux (`runuser`, `blkid`, `findmnt`), udev, a `runner` user, and an
unconfigured `/opt/actions-runner` installation plus its dependencies. The
template must have no pre-existing runner service, registration, jobs, Shaula
startup marker, or cloud-init instance state. The `ide2` bay must be empty or
contain that VM's native Proxmox cloud-init drive, with no other seed/ISO.
The provider refuses to overwrite unrelated inherited media. Template image
content and network access remain the publisher's trust responsibility;
selecting a name is not immutable image attestation.

Cloud-init writes a mandatory root-owned bootstrap and a systemd service with
`Restart=no`. The service waits for cloud-final, is started once without
blocking cloud-init, and is never enabled at boot. A durable startup marker
prevents re-registration after a manual restart or interrupted bootstrap.
An optional publisher-authored Bash preparation body runs first, without a
JIT environment or parameter substitution. It is privileged code, must not
contain credentials, and cannot replace the mandatory bootstrap contract.

JIT is base64-encoded as data in a root-owned `0600` file, never interpolated
into shell code or command arguments. Before starting the Listener, bootstrap
restricts guest cloud-init caches and the `CIDATA` block device to root,
unmounts any seed mount, reads and deletes the JIT handoff file, and drops to
the `runner` user. It runs `/opt/actions-runner/bin/Runner.Listener run` with
the official `ACTIONS_RUNNER_INPUT_JITCONFIG` input. The Listener captures and
removes the input before running the ephemeral job. It receives no Proxmox,
GitHub App, installation-token, or Shaula control-plane credentials. This
contract does not deliver the container-only completed-apply Setup Info.

The ISO remains attached and contains JIT until Terraform destroys it. ISO
storage, Terraform inputs/plans/state, provider temporary files, and raw
diagnostics are credential-grade. Guest permissions do not isolate a job
that can become root, inspect processes, or administer the hypervisor. The
runner image's privilege and sudo policies require publisher review.

The API endpoint must serve the template node when using node-local ISO
storage; cross-node uploads require actually verified shared storage. ISO
visibility checks fail closed before cloning when the target node cannot
see the exact seed. Upload and clone outcomes lost before an accepted task
identity require operator reconciliation, not blind retries or adoption.
Provider task waits are bounded at ten minutes; large full clones may require
manual recovery. Real guest DHCP, cloud-init, JIT registration, job execution,
and cleanup must be accepted on the target Proxmox environment separately.

## Forgejo cloud-init contract

`shaula.forgejo-vm-cloud-init/v1` explicitly permits `shaula.forgejo_vm.token`
in protected Terraform input and the NoCloud ISO. Shaula first registers one
ephemeral Runner and supplies its UUID/URL/labels and per-Runner token, never
the Forgejo management credential. The clean base image must be Ubuntu 22.04
x86_64 with cloud-init/systemd, util-linux and udev; it need not contain GitHub
Runner. No pre-enabled Runner services, privileged runner user, old cloud-init
state or Shaula marker are allowed. Bootstrap installs the official native
`forgejo-runner-13.1.0-linux-amd64`, pinned to SHA-256
`29dae21e93f0eab5cdf3564008d44603c74770b41a4f4f1aceed172c774bc376`.
No guest registration, custom Runner build or latest lookup is used.

The root bootstrap claims the one-use marker before preparation, protects
cloud-init caches, requires the exact CIDATA seed to be root-only and unmounted,
and copies the token into `/run/shaula-forgejo/token` (0600 inside a runner-owned
0700 directory). Failures stop launch. The initial handoff is removed. The
service has Restart=no, NoNewPrivileges=true and no boot enablement; it drops
to non-root runner. Identity JSON is data, never shell. `one-job --wait` reads
`--token-url file:...`, never a token in argv/environment. Only explicit `:host`
labels are admitted, meaning this disposable VM. Workflow tools beyond git
(such as Node) require reviewed image/preparation provisioning. Ubuntu packages,
Forgejo release assets and the selected instance need outbound reachability.

Forgejo ephemeral mode limits this registration to one job; it does not remove
the VM or seed. Shaula destroys both after exact registration absence, or the
shared hard lifetime (default 7200 seconds after Create succeeds, even Busy).
A live idle registration alone is not safe-drain proof. Failed bootstrap never
replays the seed. The remote ISO, Terraform inputs/plans/state and guest caches
remain credential-grade; permissions/base64 are not encryption or hypervisor
isolation. Management/provider credentials never enter guest materials. No
Setup Info is added; real Forgejo boot/job/VM-and-ISO cleanup needs separate
acceptance on the target Proxmox deployment.
