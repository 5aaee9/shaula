"""Ordering and refusal checks for the host-side external smoke bootstrap."""

import copy
import io
import json
import tarfile
import unittest
from unittest.mock import MagicMock, patch

from bootstrap import SMOKE_SETUP_INFO, stage_and_start
from safety import Rejected
from test_safety import container


class Commands:
    def __init__(self):
        self.info = container()
        self.info.update({
            "Id": "a" * 64,
            "State": {"Running": False, "Status": "created"},
        })
        self.info["Config"]["Labels"]["shaula.fleet"] = "smoke"
        self.events = []
        self.wrong_readback = False

    def docker_json(self, *arguments, phase):
        self.events.append(phase)
        return [self.info]

    def docker_run(self, *arguments, phase):
        self.events.append(phase)
        if phase != "bootstrap_setup_info_readback":
            return b""
        payload = (json.dumps(SMOKE_SETUP_INFO) + "\n").encode()
        if self.wrong_readback:
            payload = b"[]"
        result = io.BytesIO()
        with tarfile.open(fileobj=result, mode="w") as archive:
            member = tarfile.TarInfo(".setup_info")
            member.size = len(payload)
            archive.addfile(member, io.BytesIO(payload))
        return result.getvalue()


class BootstrapTests(unittest.TestCase):
    def setUp(self):
        self.commands = Commands()
        self.journal = {
            "container_id": "a" * 64,
            "name": "shaula-smoke-g1",
            "image_id": "sha256:abcd",
            "generation_id": "g1",
        }
        self.inputs = {"shaula": {
            "jit_config": "jit-canary",
            "generation": {"fleet_key": "smoke"},
        }}
        self.report = {"checks": {}}
        self.saved_journals = []

    def stage(self):
        def journal_saved(_path, document):
            self.commands.events.append("journal_saved")
            self.saved_journals.append(copy.deepcopy(document))

        with patch("bootstrap.save"), patch("bootstrap.save_json", journal_saved):
            stage_and_start(
                self.commands, self.journal, self.inputs, MagicMock(), self.report
            )

    def test_stopped_container_marker_is_verified_before_journaled_start(self):
        self.stage()
        self.assertEqual(self.commands.events, [
            "bootstrap_container_inspect",
            "bootstrap_setup_info_copy",
            "bootstrap_setup_info_readback",
            "journal_saved",
            "bootstrap_start",
            "journal_saved",
        ])
        self.assertTrue(self.saved_journals[0]["start_possible"])
        self.assertNotIn("bootstrap_completed", self.saved_journals[0])
        self.assertTrue(self.saved_journals[1]["bootstrap_completed"])
        self.assertNotIn("jit-canary", json.dumps(self.saved_journals))

    def test_running_or_previously_exited_container_never_restarts(self):
        for running, status in [(True, "running"), (False, "exited")]:
            self.commands.info["State"] = {"Running": running, "Status": status}
            self.commands.events = []
            with self.subTest(status=status), self.assertRaises(Rejected):
                self.stage()
            self.assertEqual(self.commands.events, ["bootstrap_container_inspect"])

    def test_changed_identity_or_labels_prevent_copy_and_start(self):
        self.commands.info["Id"] = "b" * 64
        with self.assertRaises(Rejected):
            self.stage()
        self.assertNotIn("bootstrap_setup_info_copy", self.commands.events)
        self.commands.info["Id"] = "a" * 64
        self.commands.info["Config"]["Labels"]["shaula.generation"] = "g2"
        with self.assertRaises(Rejected):
            self.stage()
        self.assertNotIn("bootstrap_setup_info_copy", self.commands.events)

    def test_wrong_readback_prevents_start_and_journaled_start_permission(self):
        self.commands.wrong_readback = True
        with self.assertRaises(Rejected):
            self.stage()
        self.assertNotIn("bootstrap_start", self.commands.events)
        self.assertFalse(self.saved_journals)

    def test_uncertain_previous_start_is_never_repeated(self):
        self.journal["start_possible"] = True
        with self.assertRaises(Rejected):
            self.stage()
        self.assertFalse(self.commands.events)


if __name__ == "__main__":
    unittest.main()
