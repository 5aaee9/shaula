"""External smoke-marker handoff; this does not exercise the production archive."""

import io
import json
import tarfile

from safety import Rejected, digest, inspect_container, require
from support import now, save, save_json


SMOKE_SETUP_INFO = [{
    "Group": "Shaula bootstrap smoke test",
    "Detail": (
        "Prepared outside the official runner container after Terraform apply completed. "
        "This smoke marker does not prove production operation-log delivery."
    ),
}]


def stage_and_start(commands, journal, inputs, evidence, report):
    """Prove the stopped gate, write and verify a safe marker, then start once."""
    require(not journal.get("start_possible"), "bootstrap_uncertain_never_restart")
    identifier = journal["container_id"]
    info = commands.docker_json(
        "container", "inspect", identifier, phase="bootstrap_container_inspect"
    )[0]
    require(info.get("Id") == identifier, "bootstrap_container_identity")
    inspect_container(
        info, journal["name"], journal["image_id"], inputs["shaula"]["jit_config"],
        expected_labels={
            "shaula.fleet": inputs["shaula"]["generation"]["fleet_key"],
            "shaula.generation": journal["generation_id"],
        },
    )
    require(
        info.get("State", {}).get("Running") is False
        and info.get("State", {}).get("Status") == "created",
        "bootstrap_container_already_started",
    )
    payload = (json.dumps(SMOKE_SETUP_INFO) + "\n").encode()
    marker = evidence / "smoke-setup-info.json"
    save(marker, payload)
    # The marker contains no inputs or credentials. The containing directory
    # remains private; 0444 makes Docker's root-owned copy readable by runner.
    marker.chmod(0o444)
    commands.docker_run(
        "cp", str(marker), identifier + ":/home/runner/.setup_info",
        phase="bootstrap_setup_info_copy",
    )
    archive = commands.docker_run(
        "cp", identifier + ":/home/runner/.setup_info", "-",
        phase="bootstrap_setup_info_readback",
    )
    try:
        with tarfile.open(fileobj=io.BytesIO(archive)) as copied:
            members = copied.getmembers()
            require(
                len(members) == 1 and members[0].isfile()
                and members[0].size == len(payload),
                "bootstrap_setup_info_shape",
            )
            require(copied.extractfile(members[0]).read() == payload, "bootstrap_setup_info_mismatch")
    except tarfile.TarError:
        raise Rejected("bootstrap_setup_info_archive_invalid") from None
    journal["setup_info_digest"] = digest(payload)
    journal["setup_info_staged_at"] = now()
    journal["start_possible"] = True
    save_json(evidence / "journal.json", journal)
    commands.docker_run("start", identifier, phase="bootstrap_start")
    journal["bootstrap_completed"] = True
    save_json(evidence / "journal.json", journal)
    report["checks"].update({
        "official_container_stopped_until_apply_completed": "passed",
        "external_setup_info_smoke_marker_before_start": "passed",
        "official_listener_started_by_host": "passed",
    })
