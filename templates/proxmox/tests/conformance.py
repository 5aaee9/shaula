"""Run with Python 3.10+, PyYAML, OpenSSL and a real Terraform/provider binary.

All API traffic goes to a temporary local TLS mock. No real VM is provisioned.
Generated plans use only synthetic credentials and can be retained for the
Shaula plan-admission tests with --output-dir.
"""

import argparse
import base64
import json
import shutil
import ssl
import subprocess
import tempfile
import threading
from pathlib import Path

import yaml
from mock_pve import MockPve


def run(args, cwd, success=True):
    result = subprocess.run(
        args,
        cwd=cwd,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
        check=False,
        timeout=120,
    )
    if (result.returncode == 0) != success:
        raise AssertionError(result.stdout + result.stderr)
    return result.stdout + result.stderr


def check_plan(plan, jit):
    resources = {
        item["address"]: item["values"]
        for item in plan["planned_values"]["root_module"]["resources"]
    }
    iso = resources["proxmox_nocloud_iso.bootstrap"]
    vm = resources["proxmox_qemu_vm.runner"]
    assert vm["node"] == iso["node"] == "pve-test"
    assert vm["clone"]["source_vmid"] == 9000 and not vm["clone"]["full"]
    assert vm["vm_id_start"] == 100 and iso["storage"] == "local"
    assert (
        vm["start_on_create"]
        and vm["stop_on_destroy"]
        and not vm["onboot"]
        and not vm["protection"]
    )
    assert vm["nocloud_cdrom_slot"] == "ide2" and list(vm["disk"]) == ["ide2"]
    assert vm["disk"]["ide2"]["media"] == "cdrom"
    assert (
        vm["name"] == "shaula-test-generation"
        and iso["filename"] == "shaula-test-generation.iso"
    )
    assert yaml.safe_load(iso["meta_data"])["instance-id"] == vm["name"]
    assert yaml.safe_load(iso["network_config"])["ethernets"]["runner"] == {
        "dhcp4": True,
        "match": {"name": "e*"},
    }
    cloud = yaml.safe_load(iso["user_data"])
    files = {item["path"]: item for item in cloud["write_files"]}
    handoff = files["/var/lib/shaula/jit-config"]
    assert base64.b64decode(handoff["content"]).decode() == jit
    assert handoff["permissions"] == "0600" and handoff["owner"] == "root:root"
    assert "synthetic=secret" not in iso["user_data"]
    assert (
        jit
        not in base64.b64decode(
            files["/usr/local/libexec/shaula-runner"]["content"]
        ).decode()
    )
    assert base64.b64decode(files["/var/lib/shaula/pre-start"]["content"]) == b""
    assert all(check["status"] == "pass" for check in plan.get("checks", []))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--terraform", required=True)
    parser.add_argument("--openssl", default="openssl")
    parser.add_argument(
        "--plugin-dir", help="Optional local Terraform filesystem provider mirror"
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        help="Retain synthetic create/destroy plans and apply state JSON",
    )
    args = parser.parse_args()
    server = MockPve()
    with tempfile.TemporaryDirectory(prefix="shaula-proxmox-conformance-") as temporary:
        root = Path(temporary)
        module = root / "module"
        shutil.copytree(
            Path(__file__).resolve().parents[1],
            module,
            ignore=shutil.ignore_patterns("tests", ".terraform"),
        )
        run(
            [
                args.openssl,
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-keyout",
                "key.pem",
                "-out",
                "cert.pem",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
            ],
            root,
        )
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(root / "cert.pem", root / "key.pem")
        server.socket = context.wrap_socket(server.socket, server_side=True)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        terraform = [args.terraform]
        init = ["init", "-backend=false", "-lockfile=readonly", "-no-color"]
        if args.plugin_dir:
            init.append(f"-plugin-dir={args.plugin_dir}")
        try:
            run(terraform + init, module)
            run(terraform + ["validate", "-no-color"], module)
            jit = "synthetic-jit-'\"-$()-not-a-real-credential"
            envelope = {
                "contract_version": 1,
                "generation": {
                    "id": "test-generation-id",
                    "generation_name": "shaula-test-generation",
                },
                "jit_config": jit,
                "bindings_digest": "synthetic-bindings-digest",
                "bindings": {
                    "proxmox_host": f"https://127.0.0.1:{server.server_port}",
                    "proxmox_token": "test@pve!shaula=synthetic=secret",
                },
                "parameters": {},
            }

            def inputs():
                (module / "inputs.auto.tfvars.json").write_text(
                    json.dumps({"shaula": envelope}), encoding="utf-8"
                )

            def plan(name, *options):
                run(
                    terraform
                    + [
                        "plan",
                        "-no-color",
                        "-input=false",
                        f"-out={name}.plan",
                        *options,
                    ],
                    module,
                )
                value = json.loads(
                    run(terraform + ["show", "-json", f"{name}.plan"], module)
                )
                if args.output_dir:
                    args.output_dir.mkdir(parents=True, exist_ok=True)
                    (args.output_dir / f"{name}.json").write_text(
                        json.dumps(value, indent=2) + "\n", encoding="utf-8"
                    )
                return value

            inputs()
            # Negative inventory goes through the actual provider's filtering.
            for inventory in ([], [server.template(9000), server.template(9001)]):
                server.inventory = inventory
                failure = run(
                    terraform + ["plan", "-no-color", "-input=false"],
                    module,
                    success=False,
                )
                assert "exactly one visible Proxmox template VM" in failure
                assert not server.events
            server.inventory = [server.template(9000)]
            for key, value in (
                ("proxmox_vmid_begin", 99),
                ("proxmox_vmid_begin", 100.5),
                ("proxmox_token", "malformed"),
                ("proxmox_host", "http://example.test:8006"),
            ):
                original = dict(envelope["bindings"])
                envelope["bindings"][key] = value
                inputs()
                run(
                    terraform + ["plan", "-no-color", "-input=false"],
                    module,
                    success=False,
                )
                assert not server.events
                envelope["bindings"] = original
            inputs()
            create = plan("create")
            check_plan(create, jit)
            # Publisher code is literal data; Terraform must not interpret the
            # shell's dollar syntax or offer it the JIT configuration.
            body = "printf '%s\\n' '${not_a_terraform_variable} $(not_a_terraform_command)'"
            envelope["bindings"]["proxmox_cloud_init_cmd"] = body
            inputs()
            custom = plan("custom-hook")
            seed = next(
                item
                for item in custom["planned_values"]["root_module"]["resources"]
                if item["address"] == "proxmox_nocloud_iso.bootstrap"
            )
            hook = next(
                item
                for item in yaml.safe_load(seed["values"]["user_data"])["write_files"]
                if item["path"] == "/var/lib/shaula/pre-start"
            )
            assert base64.b64decode(hook["content"]).decode() == body
            del envelope["bindings"]["proxmox_cloud_init_cmd"]
            inputs()
            run(
                terraform + ["apply", "-no-color", "-input=false", "create.plan"],
                module,
            )
            assert server.events == ["upload", "clone", "attach", "start"], (
                server.events
            )
            if args.output_dir:
                (args.output_dir / "state.json").write_text(
                    run(terraform + ["show", "-json"], module), encoding="utf-8"
                )
            # Shaula retains exact state and uses a saved destroy plan. The
            # managed precondition may be unknown only on deleted resources.
            # Source-template deletion must not prevent owned-resource cleanup.
            server.inventory = []
            destroy = plan("destroy", "-destroy")
            changes = [
                item
                for item in destroy["resource_changes"]
                if item["mode"] == "managed"
            ]
            assert len(changes) == 2 and all(
                item["change"]["actions"] == ["delete"] for item in changes
            )
            assert all(
                check["status"] == "pass" or check["address"].get("mode") == "managed"
                for check in destroy.get("checks", [])
            )
            run(
                terraform + ["apply", "-no-color", "-input=false", "destroy.plan"],
                module,
            )
            assert server.events == [
                "upload",
                "clone",
                "attach",
                "start",
                "stop",
                "vm-delete",
                "iso-delete",
            ], server.events
            assert not server.vms and not server.isos and not server.failures, (
                server.failures
            )
            state = json.loads(run(terraform + ["show", "-json"], module))
            assert (
                not state.get("values", {}).get("root_module", {}).get("resources", [])
            )
            print(
                "PASS: real Terraform/provider create and destroy, six negative plans, literal hook/JIT rendering, API token splitting, inherited hardware, lifecycle ordering and empty final state"
            )
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


if __name__ == "__main__":
    main()
