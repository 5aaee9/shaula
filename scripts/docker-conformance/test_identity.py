"""Bind safe removal to this JIT runner and the exact saved-plan container."""

import base64
import datetime
import json
import unittest

from safety import Rejected, admit_destroy_identity, admit_removal, runner_identity


def jit(runner):
    nested = base64.b64encode(json.dumps(runner).encode()).decode()
    return base64.b64encode(json.dumps({".runner": nested}).encode()).decode()


class IdentityTests(unittest.TestCase):
    def test_jit_freezes_actual_runner_identity(self):
        self.assertEqual(
            runner_identity(jit({"agentId": 123, "agentName": "actual"})),
            (123, "actual"),
        )
        for runner in (
            {"agentId": True, "agentName": "actual"},
            {"agentName": "actual"},
            {"agentId": 123, "agentName": ""},
        ):
            with self.assertRaises(Rejected):
                runner_identity(jit(runner))

    def test_real_shaped_pascal_case_jit_with_decimal_id(self):
        # Non-secret metadata only, matching the live GitHub serialization.
        runner = {
            "AgentId": "2",
            "AgentName": "shaula-codex-docker-smoke-20260908-01",
            "Ephemeral": True,
            "DisableUpdate": True,
            "WorkFolder": "_work",
        }
        self.assertEqual(runner_identity(jit(runner)), (2, runner["AgentName"]))

    def test_supported_aliases_and_exact_safe_integer_boundaries(self):
        for id_key in ("AgentId", "agentId"):
            for name_key in ("AgentName", "agentName"):
                for identifier in (1, "1", (1 << 53) - 1, str((1 << 53) - 1)):
                    with self.subTest(
                        id_key=id_key, name_key=name_key, identifier=identifier
                    ):
                        self.assertEqual(
                            runner_identity(
                                jit({id_key: identifier, name_key: "actual"})
                            ),
                            (int(identifier), "actual"),
                        )

    def test_ambiguous_member_spellings_rejected_even_when_equal(self):
        for runner in (
            {"AgentId": "2", "agentId": "2", "AgentName": "actual"},
            {"AgentId": "2", "agentId": "3", "AgentName": "actual"},
            {"AgentId": "2", "AgentName": "actual", "agentName": "actual"},
            {"AgentId": "2", "AgentName": "actual", "agentName": "other"},
        ):
            with self.assertRaises(Rejected):
                runner_identity(jit(runner))

    def test_noncanonical_or_invalid_ids_rejected(self):
        invalid = (
            None,
            True,
            False,
            0,
            -1,
            1.0,
            1.5,
            "",
            "0",
            "01",
            "+1",
            "-1",
            " 1",
            "1 ",
            "1.0",
            "1e2",
            "\u0661",
            1 << 53,
            str(1 << 53),
            "9" * 17,
        )
        for identifier in invalid:
            with self.subTest(identifier=identifier), self.assertRaises(Rejected):
                runner_identity(jit({"AgentId": identifier, "AgentName": "actual"}))

    def test_other_runner_absence_cannot_authorize_this_container(self):
        journal = {
            "generation_id": "g1",
            "container_id": "a" * 64,
            "runner_id": 123,
            "runner_name": "actual",
        }
        receipt = {
            **journal,
            "repository": "5aaee9/shaula",
            "busy": False,
            "absent": True,
            "method": "github-rest-get-404",
            "observed_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        }
        admit_removal(receipt, journal)
        for key, value in (
            ("runner_id", 456),
            ("runner_name", "another"),
            ("generation_id", "g2"),
            ("container_id", "b" * 64),
            ("busy", True),
            ("absent", False),
        ):
            with self.subTest(key=key), self.assertRaises(Rejected):
                admit_removal({**receipt, key: value}, journal)

    def test_stale_or_future_removal_receipt_rejected(self):
        journal = {
            "generation_id": "g1",
            "container_id": "a" * 64,
            "runner_id": 123,
            "runner_name": "actual",
        }
        for offset in (-301, 60):
            observed = datetime.datetime.now(
                datetime.timezone.utc
            ) + datetime.timedelta(seconds=offset)
            receipt = {
                **journal,
                "repository": "5aaee9/shaula",
                "busy": False,
                "absent": True,
                "method": "github-rest-get-404",
                "observed_at": observed.isoformat(),
            }
            with self.assertRaises(Rejected):
                admit_removal(receipt, journal)

    def test_same_address_different_saved_plan_id_rejected(self):
        resource = {
            "address": "docker_container.runner",
            "mode": "managed",
            "type": "docker_container",
            "change": {"actions": ["delete"], "before": {"id": "a" * 64}},
        }
        document = {"resource_changes": [resource]}
        admit_destroy_identity(document, "a" * 64)
        with self.assertRaises(Rejected):
            admit_destroy_identity(document, "b" * 64)
        resource["change"]["before"] = {}
        with self.assertRaises(Rejected):
            admit_destroy_identity(document, "a" * 64)


if __name__ == "__main__":
    unittest.main()
