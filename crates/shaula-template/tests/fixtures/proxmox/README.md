# Proxmox saved-plan admission fixtures

Captured with Terraform 1.15.8 and the locked `indexyz/proxmox` 0.7.1 binary,
using `templates/proxmox/tests/conformance.py --output-dir <temporary-directory>`
against its local HTTPS mock PVE server. All inputs are synthetic.

These files retain the actual plan format/completion flags, resource-change
identity/actions, prior-state resource identities and checks. Variables, outputs,
configuration expressions, and resource values/sensitive-value masks are omitted.
No plan flag or check status has been synthesized or changed. The lifecycle
admission test consumes this projection through the production core interface.

Create has passed variable and ISO precondition checks. Destroy was generated
with the normal refresh-enabled `plan -destroy`, after removing the source
template from the mock API inventory. Its owned ISO precondition is `unknown`
with no instance results, while the provider-bound variable check still passes.
The existing narrowly scoped deleted-resource exception admits this shape;
Create must still reject it. The test does not prove real VM boot or jobs.

Destroy fixtures also retain the refreshed VM `change.before.name`, the only
value that tells a Generation's own VM from another Generation's VM that
reused its VMID. `destroy-reused-0.5.1.json` was captured with provider 0.5.1:
the real refresh-enabled `plan -destroy` after the harness renamed the VM at
the same VMID (the VMID reuse seen on molecule, 2026-10-16) adopts and deletes
the foreign VM, so Shaula's identity admission must refuse it. From 0.7.1 the
provider drops the foreign VM on refresh: `destroy-reused.json` then has the
same shape as `destroy-vanished.json`.
`state-pull.json` is the raw `terraform state pull` document after the Create
apply, projected to each instance's `id` and `name`.
`destroy-vanished.json` is the real `plan -destroy` after the VM disappeared:
refresh drops it from both `resource_changes` and the refreshed
`prior_state` (projected to resource identity, without values), which proves
it is already gone.
