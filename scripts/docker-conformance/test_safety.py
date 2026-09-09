"""Offline regression checks for refusal before mutating Terraform operations."""

import copy
import hashlib
import unittest

from safety import (
    Rejected,
    admit_docker_engine,
    admit_plan,
    inspect_container,
    lock_providers,
)


def plan():
    return {
        "format_version": "1.2",
        "applyable": True,
        "complete": True,
        "errored": False,
        "resource_changes": [
            {
                "address": "docker_container.runner",
                "mode": "managed",
                "type": "docker_container",
                "change": {"actions": ["create"]},
            }
        ],
    }


def container():
    return {
        "Name": "/shaula-smoke-g1",
        "Image": "sha256:abcd",
        "Mounts": [],
        "Config": {
            "User": "1001",
            "Cmd": ["/home/runner/bin/Runner.Listener", "run"],
            "Entrypoint": None,
            "Labels": {"shaula.generation": "g1"},
            "Env": ["PATH=/bin", "ACTIONS_RUNNER_INPUT_JITCONFIG=jit-canary"],
        },
        "HostConfig": {
            "RestartPolicy": {"Name": "no"},
            "AutoRemove": False,
            "Privileged": False,
            "NetworkMode": "bridge",
            "IpcMode": "private",
        },
    }


def destroy_plan_with_unevaluated_check():
    document = plan()
    document["resource_changes"][0]["change"] = {
        "actions": ["delete"],
        "before": {"id": "a" * 64},
    }
    document["checks"] = [
        {
            "address": {
                "kind": "resource",
                "mode": "managed",
                "name": "runner",
                "to_display": "docker_container.runner",
                "type": "docker_container",
            },
            "status": "unknown",
        }
    ]
    return document


class SafetyTests(unittest.TestCase):
    def test_actual_destroy_unknown_resource_check_is_unevaluated(self):
        admit_plan(
            destroy_plan_with_unevaluated_check(), "delete", ["docker_container.runner"]
        )

    def test_unknown_check_never_permitted_for_create(self):
        document = destroy_plan_with_unevaluated_check()
        document["resource_changes"][0]["change"] = {"actions": ["create"]}
        with self.assertRaises(Rejected):
            admit_plan(document, "create", [])

    def test_destroy_unknown_check_requires_exact_full_owned_address(self):
        for key, value in (
            ("kind", "output"),
            ("mode", "data"),
            ("name", "other"),
            ("to_display", "docker_container.other"),
            ("type", "docker_image"),
            ("module", "module.other"),
        ):
            document = destroy_plan_with_unevaluated_check()
            document["checks"][0]["address"][key] = value
            with self.subTest(key=key), self.assertRaises(Rejected):
                admit_plan(document, "delete", ["docker_container.runner"])
        document = destroy_plan_with_unevaluated_check()
        with self.assertRaises(Rejected):
            admit_plan(document, "delete", ["docker_container.other"])

    def test_destroy_failed_problematic_or_nested_unknown_checks_rejected(self):
        for key, value in (
            ("status", "fail"),
            ("status", "error"),
            ("status", "skipped"),
            ("problems", [{}]),
            ("instances", [{"status": "fail"}]),
        ):
            document = destroy_plan_with_unevaluated_check()
            document["checks"][0][key] = value
            with self.subTest(key=key, value=value), self.assertRaises(Rejected):
                admit_plan(document, "delete", ["docker_container.runner"])

    def test_real_nixos_engine_with_empty_platform_name(self):
        server = {
            "Platform": {"Name": ""},
            "Version": "29.6.2",
            "Os": "linux",
            "Components": [
                {
                    "Name": "Engine",
                    "Version": "29.6.2",
                    "Details": {"Module": "github.com/moby/moby/v2"},
                },
                {"Name": "containerd", "Details": {}},
                {"Name": "runc", "Details": {}},
                {"Name": "docker-init", "Details": {}},
            ],
        }
        admit_docker_engine(server)
        for module in ("github.com/moby/moby", "github.com/docker/docker"):
            server["Components"][0]["Details"]["Module"] = module
            admit_docker_engine(server)

    def test_branded_docker_or_moby_platform_still_accepted(self):
        for name in ("Docker Engine - Community", "Moby Engine"):
            admit_docker_engine({"Platform": {"Name": name}, "Os": "linux"})

    def test_podman_unknown_components_or_nonlinux_rejected(self):
        invalid = [
            {"Platform": {"Name": "Podman Engine"}, "Os": "linux"},
            {"Platform": {"Name": "Docker compatible Podman"}, "Os": "linux"},
            {
                "Platform": {"Name": ""},
                "Os": "linux",
                "Components": [
                    {
                        "Name": "Engine",
                        "Details": {"Module": "github.com/containers/podman/v5"},
                    }
                ],
            },
            {
                "Platform": {"Name": ""},
                "Os": "linux",
                "Components": [
                    {"Name": "Other", "Details": {"Module": "github.com/moby/moby/v2"}}
                ],
            },
            {
                "Platform": {"Name": ""},
                "Os": "linux",
                "Components": [
                    {"Name": "Engine", "Details": {"Module": "github.com/moby/moby/v3"}}
                ],
            },
            {"Platform": {"Name": "Docker Engine - Community"}, "Os": "windows"},
            {"Platform": {"Name": ""}, "Os": "linux"},
        ]
        for server in invalid:
            with self.subTest(server=server), self.assertRaises(Rejected):
                admit_docker_engine(server)

    def test_create_accepts_single_container_and_readonly_data(self):
        document = plan()
        document["resource_changes"].append(
            {
                "address": "data.docker_image.runner",
                "mode": "data",
                "type": "docker_image",
                "change": {"actions": ["read"]},
            }
        )
        admit_plan(document, "create", [])

    def test_create_rejects_drift_incomplete_and_mutation_shapes(self):
        for key, value in (
            ("complete", False),
            ("applyable", False),
            ("errored", True),
            ("resource_drift", [{}]),
            ("deferred_changes", [{}]),
            ("format_version", "2.0"),
        ):
            with self.subTest(key=key):
                document = plan()
                document[key] = value
                with self.assertRaises(Rejected):
                    admit_plan(document, "create", [])
        for actions in (["update"], ["no-op"], ["delete", "create"], ["forget"]):
            document = plan()
            document["resource_changes"][0]["change"]["actions"] = actions
            with self.assertRaises(Rejected):
                admit_plan(document, "create", [])

    def test_create_rejects_existing_state_and_ambiguous_ownership(self):
        with self.assertRaises(Rejected):
            admit_plan(plan(), "create", ["docker_container.runner"])
        document = plan()
        document["prior_state"] = {
            "values": {
                "root_module": {"child_modules": [{"resources": [{"mode": "managed"}]}]}
            }
        }
        with self.assertRaises(Rejected):
            admit_plan(document, "create", [])
        for key in ("deposed", "importing", "previous_address"):
            document = plan()
            document["resource_changes"][0][key] = "unexpected"
            with self.assertRaises(Rejected):
                admit_plan(document, "create", [])

    def test_destroy_requires_every_exact_state_address(self):
        document = plan()
        document["resource_changes"][0]["change"]["actions"] = ["delete"]
        admit_plan(document, "delete", ["docker_container.runner"])
        for addresses in (
            [],
            ["docker_container.other"],
            ["docker_container.runner", "docker_container.other"],
        ):
            with self.assertRaises(Rejected):
                admit_plan(document, "delete", addresses)
        document["resource_changes"].append(
            copy.deepcopy(document["resource_changes"][0])
        )
        with self.assertRaises(Rejected):
            admit_plan(document, "delete", ["docker_container.runner"])

    def test_default_security_shape_accepts_and_privilege_variants_reject(self):
        inspect_container(container(), "shaula-smoke-g1", "sha256:abcd", "jit-canary")
        mutations = [
            ("Privileged", True),
            ("AutoRemove", True),
            ("Binds", ["/var/run/docker.sock:/socket"]),
            ("NetworkMode", "host"),
            ("PidMode", "container:peer"),
            ("CapAdd", ["SYS_ADMIN"]),
            ("Devices", [{"PathOnHost": "/dev/kvm"}]),
            ("RestartPolicy", {"Name": "always"}),
        ]
        for key, value in mutations:
            with self.subTest(key=key):
                document = container()
                document["HostConfig"][key] = value
                with self.assertRaises(Rejected):
                    inspect_container(
                        document, "shaula-smoke-g1", "sha256:abcd", "jit-canary"
                    )

    def test_jit_metadata_and_root_user_reject(self):
        for key, value in (
            ("Env", ["PATH=/bin"]),
            ("Env", ["ACTIONS_RUNNER_INPUT_JITCONFIG=wrong-value"]),
            ("Env", ["ACTIONS_RUNNER_INPUT_JITCONFIG=jit-canary"] * 2),
            ("Env", ["ACTIONS_RUNNER_INPUT_JITCONFIG=jit-canary", "JIT=jit-canary"]),
            ("Env", ["ACTIONS_RUNNER_INPUT_JITCONFIG=jit-canary", "GH_TOKEN=secret"]),
            (
                "Env",
                [
                    "ACTIONS_RUNNER_INPUT_JITCONFIG=jit-canary",
                    "actions_runner_input_token=secret",
                ],
            ),
            ("Labels", {"shaula.generation": "jit-canary"}),
            ("Cmd", ["/usr/local/bin/bootstrap-shim"]),
            (
                "Cmd",
                [
                    "/home/runner/bin/Runner.Listener",
                    "run",
                    "--jitconfig",
                    "jit-canary",
                ],
            ),
            ("User", "root"),
            ("User", "root:1001"),
            ("User", "000:1001"),
            ("Entrypoint", ["/wrapper"]),
        ):
            document = container()
            document["Config"][key] = value
            with self.assertRaises(Rejected):
                inspect_container(
                    document, "shaula-smoke-g1", "sha256:abcd", "jit-canary"
                )

    def test_generation_labels_must_match_when_supplied(self):
        expected = {"shaula.generation": "g1"}
        inspect_container(
            container(),
            "shaula-smoke-g1",
            "sha256:abcd",
            "jit-canary",
            expected_labels=expected,
        )
        document = container()
        document["Config"]["Labels"]["shaula.generation"] = "g2"
        with self.assertRaises(Rejected):
            inspect_container(
                document,
                "shaula-smoke-g1",
                "sha256:abcd",
                "jit-canary",
                expected_labels=expected,
            )

    def test_provider_commitment_has_sorted_hashes_and_final_newline(self):
        hashes = ["zh:" + "b" * 64, "zh:" + "a" * 64]
        lock = (
            'provider "registry.terraform.io/kreuzwerker/docker" {\n version = "3.0.2"\n hashes = ['
            + ",".join('"' + value + '"' for value in hashes)
            + "]\n}\n"
        ).encode()
        selected = {"registry.terraform.io/kreuzwerker/docker": "3.0.2"}
        result = lock_providers(lock, selected)
        expected = (
            "sha256:"
            + hashlib.sha256(
                "".join(value + "\n" for value in sorted(hashes)).encode()
            ).hexdigest()
        )
        self.assertEqual(
            result,
            [
                {
                    "source": "kreuzwerker/docker",
                    "version": "3.0.2",
                    "checksums_digest": expected,
                }
            ],
        )
        with self.assertRaises(Rejected):
            lock_providers(lock, {"registry.terraform.io/kreuzwerker/docker": "3.0.3"})


if __name__ == "__main__":
    unittest.main()
