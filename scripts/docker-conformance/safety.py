"""Fail-closed checks for the external Docker smoke harness, not daemon policy."""

import base64
import copy
import datetime
import hashlib
import json
import re


class Rejected(Exception):
    """A bounded diagnostic safe to expose without provider output or inputs."""


def require(condition, reason):
    if not condition:
        raise Rejected(reason)


def digest(data):
    return "sha256:" + hashlib.sha256(data).hexdigest()


def admit_docker_engine(server):
    require(isinstance(server, dict), "real_docker_engine_required")
    platform = server.get("Platform") or {}
    name = platform.get("Name", "") if isinstance(platform, dict) else ""
    require(isinstance(name, str), "real_docker_engine_required")
    components = server.get("Components") or []
    require(isinstance(components, list), "real_docker_engine_required")
    modules = {
        "github.com/moby/moby/v2",
        "github.com/moby/moby",
        "github.com/docker/docker",
    }
    known_engine = False
    markers = [name]
    for component in components:
        require(isinstance(component, dict), "real_docker_engine_required")
        details = component.get("Details") or {}
        require(isinstance(details, dict), "real_docker_engine_required")
        component_name, module = component.get("Name", ""), details.get("Module", "")
        require(
            isinstance(component_name, str) and isinstance(module, str),
            "real_docker_engine_required",
        )
        markers.extend((component_name, module))
        known_engine |= component_name == "Engine" and module in modules
    require(
        not any("podman" in marker.casefold() for marker in markers),
        "real_docker_engine_required",
    )
    # NixOS's real Engine may leave Platform.Name empty. Its Engine component
    # identifies the exact Moby module independently of the marketing label.
    require(
        "Docker" in name or "Moby" in name or known_engine,
        "real_docker_engine_required",
    )
    require(server.get("Os") == "linux", "linux_docker_engine_required")


def runner_identity(encoded):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, "jit_duplicate_member")
            result[key] = value
        return result

    require(isinstance(encoded, str) and 0 < len(encoded) <= 65536, "jit_size")
    envelope = json.loads(
        base64.b64decode(encoded, validate=True), object_pairs_hook=unique
    )
    runner = json.loads(
        base64.b64decode(envelope[".runner"], validate=True), object_pairs_hook=unique
    )
    require(isinstance(runner, dict), "jit_runner_object")

    def member(canonical, alternative):
        present = [key for key in (canonical, alternative) if key in runner]
        require(len(present) == 1, "jit_runner_member_missing_or_ambiguous")
        return runner[present[0]]

    identifier = member("AgentId", "agentId")
    name = member("AgentName", "agentName")
    # GitHub's actual JIT Runner settings serialize AgentId as decimal text.
    # Normalize only canonical, exactly representable positive IDs; accepting
    # both aliases at once would let two consumers select different identities.
    if isinstance(identifier, str):
        require(re.fullmatch(r"[1-9][0-9]{0,15}", identifier), "jit_runner_id")
        identifier = int(identifier)
    require(
        type(identifier) is int and 0 < identifier <= (1 << 53) - 1, "jit_runner_id"
    )
    require(isinstance(name, str) and 0 < len(name) <= 128, "jit_runner_name")
    return identifier, name


def admit_removal(receipt, journal):
    for key in ("generation_id", "container_id", "runner_id", "runner_name"):
        require(receipt.get(key) == journal[key], "removal_" + key + "_mismatch")
    require(type(receipt.get("runner_id")) is int, "removal_runner_identity")
    require(
        receipt.get("busy") is False and receipt.get("absent") is True,
        "github_removal_not_safe",
    )
    require(receipt.get("repository") == "5aaee9/shaula", "removal_repository_mismatch")
    require(
        receipt.get("observed_at")
        and receipt.get("method") in ("github-rest-get-404", "github-rest-inventory"),
        "removal_observation_missing",
    )
    observed = datetime.datetime.fromisoformat(
        receipt["observed_at"].replace("Z", "+00:00")
    )
    require(observed.tzinfo is not None, "removal_observation_timezone")
    age = (datetime.datetime.now(datetime.timezone.utc) - observed).total_seconds()
    require(0 <= age <= 300, "removal_observation_not_fresh")


def admit_destroy_identity(plan, container_id):
    resources = [
        resource
        for resource in plan.get("resource_changes", [])
        if resource.get("mode") == "managed"
    ]
    require(len(resources) == 1, "destroy_plan_identity_shape")
    resource = resources[0]
    require(
        resource.get("type") == "docker_container"
        and resource.get("address") == "docker_container.runner",
        "destroy_plan_identity_address",
    )
    before = resource.get("change", {}).get("before")
    require(
        isinstance(before, dict) and before.get("id") == container_id,
        "destroy_plan_container_mismatch",
    )


def managed_resources(document):
    def visit(module):
        resources = module.get("resources", [])
        require(isinstance(resources, list), "malformed_state_resources")
        result = [
            resource for resource in resources if resource.get("mode") == "managed"
        ]
        for child in module.get("child_modules", []):
            result.extend(visit(child))
        return result

    return visit(document.get("values", {}).get("root_module", {}))


def admit_plan(plan, operation, bound_addresses):
    require(str(plan.get("format_version", "")).split(".")[0] == "1", "plan_format")
    require(plan.get("applyable") is True, "plan_not_applyable")
    require(plan.get("complete") is True, "plan_incomplete")
    require(plan.get("errored") is False, "plan_errored")
    require(not plan.get("deferred_changes"), "plan_deferred")
    require(not plan.get("resource_drift"), "plan_drift")
    addresses = []
    for resource in plan.get("resource_changes", []):
        change = resource.get("change", {})
        for key in ("deposed", "importing", "previous_address"):
            require(
                resource.get(key) is None and change.get(key) is None, "plan_ambiguous"
            )
        require(change.get("action_reason") != "import", "plan_import")
        if resource.get("mode") == "data":
            require(change.get("actions") in (["read"], ["no-op"]), "plan_data_action")
            continue
        require(resource.get("mode") == "managed", "plan_unknown_mode")
        require(resource.get("type") == "docker_container", "plan_managed_type")
        require(change.get("actions") == [operation], "plan_managed_action")
        addresses.append(resource.get("address"))
    require(len(addresses) == len(set(addresses)), "plan_duplicate_address")
    if operation == "create":
        require(not bound_addresses, "create_state_not_empty")
        require(
            not managed_resources(plan.get("prior_state", {})), "create_prior_state"
        )
        require(addresses == ["docker_container.runner"], "create_shape")
    else:
        require(sorted(addresses) == sorted(bound_addresses), "destroy_state_mismatch")
    for check in plan.get("checks", []):
        require(not check.get("problems"), "plan_check")
        if check.get("status") == "pass":
            continue
        # Terraform 1.9.8's destroy node does not evaluate resource conditions:
        # node_resource_plan_destroy.go: managedResourceExecute calls
        # planDestroy/checkPreventDestroy, not the ordinary condition evaluator.
        # Its JSON can retain an unknown (unevaluated) check for the resource
        # being deleted. This exception is only for the bundled exact root
        # resource after delete-only actions and complete state coverage passed.
        expected = {
            "kind": "resource",
            "mode": "managed",
            "name": "runner",
            "to_display": "docker_container.runner",
            "type": "docker_container",
        }
        require(
            operation == "delete"
            and addresses == ["docker_container.runner"]
            and bound_addresses == ["docker_container.runner"]
            and check.get("status") == "unknown"
            and check.get("address") == expected
            and set(check) <= {"status", "address", "problems"},
            "plan_check",
        )


def inspect_container(
    container, expected_name, expected_image_id, jit, *, expected_labels=None
):
    config = container.get("Config", {})
    host = container.get("HostConfig", {})
    require(container.get("Name") == "/" + expected_name, "container_name")
    require(container.get("Image") == expected_image_id, "container_image")
    require(host.get("RestartPolicy", {}).get("Name") == "no", "container_restart")
    require(host.get("AutoRemove") is False, "container_autoremove")
    require(host.get("Privileged") is False, "container_privileged")
    for key in ("PidMode", "IpcMode", "NetworkMode", "UTSMode", "UsernsMode"):
        mode = host.get(key, "") or ""
        require(
            mode != "host" and not mode.startswith("container:"),
            "container_host_namespace",
        )
    for key in ("Devices", "DeviceRequests", "Binds", "VolumesFrom"):
        require(not host.get(key), "container_host_resource")
    require(not container.get("Mounts"), "container_mount")
    require(not host.get("CapAdd"), "container_added_capability")
    user = str(config.get("User") or "").split(":", 1)[0]
    require(
        user not in ("", "root") and not re.fullmatch(r"0+", user),
        "container_root_user",
    )
    require(
        config.get("Cmd") == ["/home/runner/bin/Runner.Listener", "run"],
        "container_command",
    )
    require(not config.get("Entrypoint"), "container_entrypoint")
    require(config.get("Labels"), "container_ownership_labels_missing")
    require(
        any(key.startswith("shaula.") for key in config["Labels"]),
        "container_ownership_label",
    )
    if expected_labels is not None:
        require(
            all(
                config["Labels"].get(key) == value
                for key, value in expected_labels.items()
            ),
            "container_ownership_labels_mismatch",
        )
    environment = config.get("Env", [])
    require(isinstance(environment, list), "container_environment_shape")
    require(
        all(isinstance(entry, str) for entry in environment),
        "container_environment_shape",
    )
    native_jit = "ACTIONS_RUNNER_INPUT_JITCONFIG=" + jit
    require(environment.count(native_jit) == 1, "container_native_jit_input")
    for entry in environment:
        if entry == native_jit:
            continue
        name = entry.split("=", 1)[0].upper()
        require(
            not name.startswith(
                (
                    "ACTIONS_RUNNER_INPUT_",
                    "TF_HTTP_",
                    "TF_VAR_",
                    "GITHUB_TOKEN",
                    "GH_TOKEN",
                    "GITHUB_APP_PRIVATE_KEY",
                    "AWS_SECRET_ACCESS_KEY",
                )
            ),
            "container_secret_environment",
        )
    # The one native input is the explicit exception. Reject every additional
    # occurrence, including another environment key, label, argv or host path.
    filtered = copy.deepcopy(container)
    filtered["Config"]["Env"].remove(native_jit)
    require(jit not in json.dumps(filtered), "container_metadata_jit")


def lock_providers(lock_bytes, selected):
    """Project Terraform's validated lock with the Rust newline-sorted hash rule."""
    text = lock_bytes.decode("utf-8")
    blocks = re.findall(r'provider\s+"([^"]+)"\s*\{([^}]+)\}', text, re.DOTALL)
    result = []
    for source, block in blocks:
        version = re.search(r'\bversion\s*=\s*"([^"]+)"', block)
        hashes = re.search(r"\bhashes\s*=\s*\[([^]]*)\]", block, re.DOTALL)
        require(version is not None and hashes is not None, "provider_lock_shape")
        values = re.findall(r'"([^"]+)"', hashes.group(1))
        require(
            values and selected.get(source) == version.group(1),
            "provider_lock_selection",
        )
        for value in values:
            require(
                re.fullmatch(r"(?:zh:[a-f0-9]{64}|h1:[A-Za-z0-9+/]{43}=)", value),
                "provider_lock_checksum",
            )
        result.append(
            {
                "source": source.removeprefix("registry.terraform.io/"),
                "version": version.group(1),
                "checksums_digest": digest(
                    "".join(value + "\n" for value in sorted(values)).encode()
                ),
            }
        )
    require(len(blocks) == len(selected) and bool(blocks), "provider_lock_count")
    return sorted(result, key=lambda provider: provider["source"])
