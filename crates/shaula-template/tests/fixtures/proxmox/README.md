# Proxmox saved-plan admission fixtures

Captured with Terraform 1.9.8 and the locked `indexyz/proxmox` 0.4.0 binary,
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
