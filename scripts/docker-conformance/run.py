#!/usr/bin/env python3
"""Run an external Terraform/Docker/GitHub smoke test without activating a Profile."""

import argparse
import json
import os
import re
import sys
from pathlib import Path

from bootstrap import stage_and_start
from safety import (
    Rejected,
    admit_destroy_identity,
    admit_docker_engine,
    admit_plan,
    admit_removal,
    digest,
    inspect_container,
    lock_providers,
    managed_resources,
    require,
    runner_identity,
)
from support import (
    Commands,
    now,
    private_bytes,
    private_directory,
    private_json,
    save,
    save_json,
    source_digest,
)


def prepare(args, commands, workspace, evidence, report):
    journal_path = evidence / "journal.json"
    require(not journal_path.exists(), "create_already_prepared_never_reapply")
    require(not (workspace / "terraform.tfstate").exists(), "fresh_workspace_required")
    require(
        not list(workspace.glob("*.tfvars*")), "workspace_must_not_auto_load_inputs"
    )
    frozen = private_bytes(Path(args.vars))
    inputs = json.loads(frozen)
    require(set(inputs) == {"shaula"}, "input_envelope")
    shaula = inputs["shaula"]
    require(
        isinstance(shaula.get("jit_config"), str) and shaula["jit_config"],
        "jit_missing",
    )
    require(
        shaula.get("bindings", {}).get("docker_host") == args.socket,
        "docker_binding_mismatch",
    )
    generation = shaula["generation"]
    require(re.fullmatch(r"[a-zA-Z0-9_-]{1,100}", generation["id"]), "generation_id")
    require(re.fullmatch(r"[a-z0-9-]{1,63}", generation["fleet_key"]), "fleet_key")
    runner_id, runner_name = runner_identity(shaula["jit_config"])
    require(runner_name == generation["runner_name"], "jit_runner_name_mismatch")
    image = args.image
    require(
        image and re.fullmatch(
            r"ghcr\.io/actions/actions-runner(?::[A-Za-z0-9_.-]+)?@sha256:[a-f0-9]{64}",
            image,
        ),
        "pinned_official_image_required",
    )
    manifest = (workspace / "profile.yaml").read_text()
    require(image in manifest, "image_not_in_manifest")
    require(
        "container_bootstrap_contract: shaula.container-bootstrap/v1" in manifest,
        "container_bootstrap_contract_required",
    )
    journal = {
        "generation_id": generation["id"],
        "source_digest": source_digest(workspace),
        "input_digest": digest(frozen),
        "create_possible": False,
        "image": image,
        "engine_digest": digest(Path(commands.terraform).read_bytes()),
        "runner_id": runner_id,
        "runner_name": runner_name,
        "name": "shaula-" + generation["fleet_key"] + "-" + generation["id"][:24],
    }
    save(evidence / "input.tfvars.json", frozen)
    save_json(journal_path, journal)
    report.update(
        {
            "kind": "shaula-docker-external-smoke/v1",
            "started_at": now(),
            "full_conformance_passed": False,
            "runtime": "external Terraform subprocess with protected local state",
            "source_digest": journal["source_digest"],
            "runner_image": image,
            "generation_id": generation["id"],
            "checks": {},
            "not_proven": [
                "shaula exec Driver and independent job worker",
                "daemon HTTP state backend and crash recovery",
                "GitHub online, job success and workflow environment (separate evidence)",
                "management HTTP, audit, log and telemetry redaction",
                "production operation-log archive and Setup Info projection delivery",
            ],
            "accepted_limitations": [
                "same-identity IaC children share Docker host authority",
                "Runner process inspection may reveal JIT; no memory zeroization",
                "Terraform inputs, saved plans and state remain credential-grade",
            ],
        }
    )
    server = commands.docker_json(
        "version", "--format", "{{json .Server}}", phase="docker_engine_version"
    )
    admit_docker_engine(server)
    report["docker_engine"] = {
        "version": server.get("Version"),
        "api_version": server.get("ApiVersion"),
        "minimum_api_version": server.get("MinAPIVersion"),
        "os": "linux",
    }
    daemon_id = commands.docker_json(
        "info", "--format", "{{json .ID}}", phase="docker_daemon_identity"
    )
    require(
        isinstance(daemon_id, str) and 0 < len(daemon_id) <= 128,
        "docker_daemon_identity_missing",
    )
    journal["docker_daemon_id"] = daemon_id
    image_info = commands.docker_json("image", "inspect", image, phase="image_inspect")[
        0
    ]
    journal["image_id"] = image_info["Id"]
    commands.tf("init", "-input=false", "-lockfile=readonly", phase="terraform_init")
    validation = commands.tf_json("validate", "-json", phase="terraform_validate")
    require(validation.get("valid") is True, "terraform_validate_invalid")
    version = commands.tf_json("version", "-json", phase="terraform_version")
    lock = (workspace / ".terraform.lock.hcl").read_bytes()
    report["engine"] = {
        "kind": "terraform",
        "version": version["terraform_version"],
        "binary_digest": journal["engine_digest"],
    }
    report["dependency_lock_digest"] = digest(lock)
    report["providers"] = lock_providers(lock, version.get("provider_selections", {}))
    require(
        source_digest(workspace) == journal["source_digest"], "init_changed_artifact"
    )
    commands.tf(
        "plan",
        "-input=false",
        "-lock-timeout=30s",
        "-var-file=.docker-conformance/input.tfvars.json",
        "-out=.docker-conformance/create.tfplan",
        phase="create_plan",
    )
    plan = commands.tf_json(
        "show", "-json", ".docker-conformance/create.tfplan", phase="create_plan_show"
    )
    admit_plan(plan, "create", [])
    resource = next(
        item for item in plan["resource_changes"] if item["mode"] == "managed"
    )
    after = resource["change"]["after"]
    require(
        after.get("must_run") is False and after.get("rm") is False
        and after.get("start") is False,
        "template_lifecycle_flags",
    )
    require(
        after.get("command") == ["/home/runner/bin/Runner.Listener", "run"]
        and after.get("env") == ["ACTIONS_RUNNER_INPUT_JITCONFIG=" + shaula["jit_config"]]
        and not after.get("upload"),
        "template_native_bootstrap_required",
    )
    require(after.get("image") == journal["image_id"], "plan_image_mismatch")
    journal["create_possible"] = True
    save_json(journal_path, journal)
    report["checks"]["init_validate_saved_create_plan"] = "passed"
    save_json(Path(args.report), report)
    commands.tf(
        "apply",
        "-input=false",
        "-lock-timeout=30s",
        ".docker-conformance/create.tfplan",
        phase="create_apply",
    )
    state = commands.tf_json("show", "-json", phase="create_state")
    resources = managed_resources(state)
    require(
        len(resources) == 1 and resources[0]["address"] == "docker_container.runner",
        "created_state_shape",
    )
    journal["container_id"] = resources[0]["values"]["id"]
    require(
        re.fullmatch(r"[a-f0-9]{64}", journal["container_id"]), "container_id_shape"
    )
    journal["create_completed"] = True
    save_json(journal_path, journal)
    report["container_id"] = journal["container_id"]
    report["checks"]["terraform_created_one_container"] = "passed"
    stage_and_start(commands, journal, inputs, evidence, report)


def inspect(args, commands, journal, inputs, report):
    require(journal.get("bootstrap_completed") is True, "bootstrap_not_completed")
    info = commands.docker_json(
        "container", "inspect", journal["container_id"], phase="container_inspect"
    )[0]
    inspect_container(
        info, journal["name"], journal["image_id"], inputs["shaula"]["jit_config"],
        expected_labels={
            "shaula.fleet": inputs["shaula"]["generation"]["fleet_key"],
            "shaula.generation": journal["generation_id"],
        },
    )
    require(
        info.get("State", {}).get("Running") is True,
        "container_not_running_for_handoff_check",
    )
    argv = commands.docker_run(
        "top", journal["container_id"], "-eo", "pid,args", phase="process_argv"
    )
    require(inputs["shaula"]["jit_config"].encode() not in argv, "process_argv_jit")
    require(b"Runner.Listener" in argv, "runner_listener_not_observed")
    report["checks"].update(
        {
            "docker_security_shape": "passed",
            "jit_only_in_native_input_environment": "passed",
            "jit_absent_from_process_argv": "passed",
        }
    )
    report["inspected_at"] = now()


def destroy(args, commands, workspace, evidence, journal, report):
    require(
        not journal.get("destroy_possible"),
        "destroy_uncertain_manual_recovery_required",
    )
    require(args.removal_evidence is not None, "github_removal_evidence_required")
    receipt_bytes = private_bytes(Path(args.removal_evidence))
    receipt = json.loads(receipt_bytes)
    admit_removal(receipt, journal)
    state = commands.tf_json("show", "-json", phase="destroy_state_before")
    resources = managed_resources(state)
    require(
        len(resources) == 1 and resources[0]["values"]["id"] == journal["container_id"],
        "destroy_state_ownership",
    )
    commands.tf(
        "plan",
        "-destroy",
        "-input=false",
        "-lock-timeout=30s",
        "-var-file=.docker-conformance/input.tfvars.json",
        "-out=.docker-conformance/destroy.tfplan",
        phase="destroy_plan",
    )
    plan = commands.tf_json(
        "show", "-json", ".docker-conformance/destroy.tfplan", phase="destroy_plan_show"
    )
    admit_plan(plan, "delete", [resource["address"] for resource in resources])
    admit_destroy_identity(plan, journal["container_id"])
    save(evidence / "removal-evidence.json", receipt_bytes)
    journal["destroy_possible"] = True
    save_json(evidence / "journal.json", journal)
    commands.tf(
        "apply",
        "-input=false",
        "-lock-timeout=30s",
        ".docker-conformance/destroy.tfplan",
        phase="destroy_apply",
    )
    state = commands.tf_json("show", "-json", phase="destroy_state_after")
    require(not managed_resources(state), "destroy_managed_state_not_empty")
    require(
        not commands.tf("state", "list", phase="destroy_state_list").strip(),
        "destroy_state_not_empty",
    )
    containers = commands.docker_run(
        "container",
        "ls",
        "--all",
        "--no-trunc",
        "--quiet",
        "--filter",
        "id=" + journal["container_id"],
        phase="destroy_container_absence",
    )
    require(not containers.strip(), "destroy_container_still_present")
    journal["destroy_completed"] = True
    save_json(evidence / "journal.json", journal)
    report["checks"].update(
        {
            "safe_github_removal_receipt": "operator_observed",
            "saved_delete_only_plan_and_apply": "passed",
            "empty_state_and_exact_container_absence": "passed",
        }
    )
    report["removal_evidence_digest"] = digest(receipt_bytes)
    report["completed_at"] = now()
    report["smoke_lifecycle_completed"] = True


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("phase", choices=("prepare", "inspect", "destroy"))
    for name in ("workspace", "terraform", "docker", "socket", "report"):
        parser.add_argument("--" + name, required=True)
    parser.add_argument("--vars", help="Owner-only JSON input; required for prepare")
    parser.add_argument(
        "--image", help="Exact manifest image including digest; required for prepare"
    )
    parser.add_argument(
        "--removal-evidence", help="Owner-only sanitized GitHub absence receipt"
    )
    args = parser.parse_args()
    os.umask(0o077)
    require(sys.platform.startswith("linux"), "linux_required")
    import fcntl

    workspace = private_directory(args.workspace)
    private_directory(Path(args.report).absolute().parent)
    evidence = workspace / ".docker-conformance"
    evidence.mkdir(mode=0o700, exist_ok=True)
    private_directory(evidence)
    report_path = Path(args.report).resolve()
    require(
        not report_path.is_relative_to(workspace)
        or report_path.is_relative_to(evidence),
        "report_must_be_outside_template_or_inside_evidence",
    )
    with (evidence / "harness.lock").open("a") as mutex:
        fcntl.flock(mutex, fcntl.LOCK_EX | fcntl.LOCK_NB)
        commands = Commands(workspace, args.terraform, args.docker, args.socket)
        report = private_json(Path(args.report)) if Path(args.report).exists() else {}
        try:
            if args.phase == "prepare":
                require(args.vars is not None, "protected_vars_required")
                prepare(args, commands, workspace, evidence, report)
            else:
                journal = private_json(evidence / "journal.json")
                require(
                    journal.get("create_completed") is True,
                    "create_uncertain_manual_recovery_required",
                )
                require(not journal.get("destroy_completed"), "already_destroyed")
                require(
                    source_digest(workspace) == journal["source_digest"],
                    "template_changed",
                )
                require(
                    digest(Path(commands.terraform).read_bytes())
                    == journal["engine_digest"],
                    "engine_changed",
                )
                frozen = private_bytes(evidence / "input.tfvars.json")
                require(
                    digest(frozen) == journal["input_digest"], "original_input_changed"
                )
                inputs = json.loads(frozen)
                require(
                    args.socket == inputs["shaula"]["bindings"]["docker_host"],
                    "docker_binding_mismatch",
                )
                daemon_id = commands.docker_json(
                    "info", "--format", "{{json .ID}}", phase="docker_daemon_identity"
                )
                require(
                    daemon_id == journal["docker_daemon_id"], "docker_daemon_changed"
                )
                if args.phase == "inspect":
                    inspect(args, commands, journal, inputs, report)
                else:
                    destroy(args, commands, workspace, evidence, journal, report)
            report["last_phase"] = args.phase
            report["last_result"] = "passed"
            report.pop("reason", None)
        except (
            Rejected,
            OSError,
            ValueError,
            KeyError,
            TypeError,
            AttributeError,
            IndexError,
            RecursionError,
        ) as error:
            report["last_phase"] = args.phase
            report["last_result"] = "failed"
            report["reason"] = (
                str(error)
                if isinstance(error, Rejected)
                else "harness_input_or_io_error"
            )
            save_json(Path(args.report), report)
            raise Rejected(report["reason"]) from None
        save_json(Path(args.report), report)
        print(
            json.dumps(
                {
                    "phase": args.phase,
                    "result": "passed",
                    "full_conformance_passed": False,
                }
            )
        )


if __name__ == "__main__":
    try:
        main()
    except (
        Rejected,
        OSError,
        ValueError,
        KeyError,
        TypeError,
        AttributeError,
        IndexError,
        RecursionError,
    ) as error:
        reason = (
            str(error) if isinstance(error, Rejected) else "harness_input_or_io_error"
        )
        print(json.dumps({"result": "failed", "reason": reason}), file=sys.stderr)
        sys.exit(1)
