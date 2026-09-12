# Proxmox template conformance

Run from the repository root with Python 3.10+, PyYAML, OpenSSL, and Terraform
1.9 or later. The real locked `indexyz/proxmox` 0.5.0 plugin is used. Terraform
initialization requires registry access unless `--plugin-dir` points to a
local filesystem provider mirror containing that locked release.

```sh
python -m pip install PyYAML==6.0.2
python templates/proxmox/tests/conformance.py --terraform terraform
```

`--openssl` accepts a custom executable path. `--output-dir` optionally saves
Full JSON of the actual create/destroy plans and applied state for
testing Shaula's plan admission. These fixtures contain synthetic JIT and API
token values only. The test creates a temporary TLS certificate and module,
binds its mock PVE server to loopback on a random port, and cleans up on exit.
It never calls a real Proxmox host or provisions a real VM.

The harness verifies actual provider filtering for missing and duplicate
templates, malformed bindings, defaults, literal custom-script and JIT
rendering, API token splitting at the first equals sign, create/stop/delete
ordering, Fleet-selected hardware, and cleanup after the source template disappears.
It also checks the final Terraform state contains no retained resources.
These are Terraform/API composition tests; they do not prove a guest obtained
DHCP, ran cloud-init, registered with GitHub, or completed a workflow.

The guest shell contract has a separate fixture under
`scripts/proxmox-conformance/`. It executes the bootstrap with controlled
guest utilities and a fake Listener. Real-image acceptance remains necessary.
