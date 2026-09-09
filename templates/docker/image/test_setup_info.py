"""Credential-free validation of setup-log transport and atomic publication."""

import importlib.util
import json
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

spec = importlib.util.spec_from_file_location(
    "setup_info", Path(__file__).with_name("setup_info.py")
)
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


def descriptor():
    return {
        "status": "enabled",
        "url": "https://example.test/runner/v1/generations/abc/setup-info",
        "capability": "a" * 64,
        "expires_at": int(time.time()) + 3600,
        "wait_seconds": 60,
    }


class ValidationTests(unittest.TestCase):
    def test_only_explicit_supported_descriptor_and_https(self):
        self.assertEqual(
            setup.validate_descriptor({"status": "disabled"}), {"status": "disabled"}
        )
        setup.validate_descriptor(descriptor())
        for url in (
            "http://example.test/x",
            "https://u:p@example.test/x",
            "https://example.test/runner/v1/generations/abc/setup-info?token=x",
            "https://example.test/runner/v1/generations/abc/setup-info#x",
            "https://example.test/runner/v1/generations/../setup-info",
        ):
            with self.subTest(url=url), self.assertRaises(ValueError):
                setup.validate_descriptor(dict(descriptor(), url=url))
        for change in (
            {"wait_seconds": 301},
            {"expires_at": 1},
            {"capability": "short"},
            {"wait_seconds": True},
            {"unexpected": 1},
        ):
            with self.subTest(change=change), self.assertRaises(ValueError):
                setup.validate_descriptor(dict(descriptor(), **change))

    def test_redirect_never_returns_a_followup_request(self):
        with self.assertRaises(ValueError):
            setup.NoRedirect().redirect_request(
                None, None, 302, "", {}, "https://other.test"
            )

    def test_rejects_oversize_wrong_group_duplicate_keys_and_nonarray(self):
        for raw in (
            b" " * (setup.MAX_BYTES + 1),
            b"{}",
            b'[{"Group":"a","Detail":"x"}]',
            b'[{"Group":"a","Group":"b","Detail":"x"}]',
        ):
            with self.subTest(size=len(raw)), self.assertRaises(ValueError):
                setup.parse_entries(raw, own_only=True)


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux file/exec semantics")
class FileTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.path = self.root / "setup_info.json"
        self.path.write_text(json.dumps(descriptor()))
        self.incoming = [{"Group": setup.GROUP, "Detail": "safe apply output"}]

    def test_atomic_merge_preserves_other_groups_and_replaces_own(self):
        path = self.root / ".setup_info"
        original = [
            {"Group": "Image", "Detail": "pinned"},
            {"Detail": "default group"},
            {"Group": setup.GROUP, "Detail": "old"},
        ]
        path.write_text(json.dumps(original))
        setup.merge_entries(self.root, self.incoming)
        setup.merge_entries(self.root, self.incoming)
        self.assertEqual(json.loads(path.read_text()), original[:2] + self.incoming)
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        self.assertFalse((self.root / setup.TEMP_NAME).exists())

    def test_symlink_and_corrupt_existing_file_are_preserved(self):
        path = self.root / ".setup_info"
        target = self.root / "target"
        target.write_text("unchanged")
        path.symlink_to(target)
        with self.assertRaises(OSError):
            setup.merge_entries(self.root, self.incoming)
        self.assertEqual(target.read_text(), "unchanged")
        path.unlink()
        path.write_text("malformed")
        with self.assertRaises(ValueError):
            setup.merge_entries(self.root, self.incoming)
        self.assertEqual(path.read_text(), "malformed")

    def test_descriptor_is_removed_before_network_and_timeout_is_bounded(self):
        def check_child(*args, **kwargs):
            self.assertFalse(self.path.exists())
            self.assertLessEqual(kwargs["timeout"], 60)
            self.assertNotIn("a" * 64, str(args))
            raise subprocess.TimeoutExpired("worker", kwargs["timeout"])

        with mock.patch.object(setup.subprocess, "run", side_effect=check_child):
            self.assertFalse(setup.prepare(self.path, self.root))
        self.assertFalse(self.path.exists())

    def test_malformed_or_symlink_descriptor_is_removed_without_target_read(self):
        self.path.write_text("invalid")
        self.assertFalse(setup.prepare(self.path, self.root))
        self.assertFalse(self.path.exists())
        target = self.root / "credential-target"
        target.write_text("unchanged")
        self.path.symlink_to(target)
        self.assertFalse(setup.prepare(self.path, self.root))
        self.assertFalse(self.path.is_symlink())
        self.assertEqual(target.read_text(), "unchanged")

    def test_hard_deadline_kills_a_stuck_worker(self):
        self.path.write_text(json.dumps(dict(descriptor(), wait_seconds=1)))
        slow_worker = self.root / "slow_worker.py"
        slow_worker.write_text("import time; time.sleep(60)")
        started = time.monotonic()
        with mock.patch.object(setup, "__file__", str(slow_worker)):
            self.assertFalse(setup.prepare(self.path, self.root))
        self.assertLess(time.monotonic() - started, 3)
        self.assertFalse(self.path.exists())


if __name__ == "__main__":
    unittest.main()
