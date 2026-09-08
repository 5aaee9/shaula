"""Source-integrity and bounded protected subprocess-diagnostic regressions."""

import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path

from safety import Rejected
from support import Commands, source_digest


class SourceTests(unittest.TestCase):
    def test_helper_bytes_and_paths_are_committed(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            helper = root / "helper"
            helper.write_bytes(b"original")
            original = source_digest(root)
            helper.write_bytes(b"changed")
            self.assertNotEqual(source_digest(root), original)
            helper.write_bytes(b"original")
            helper.rename(root / "renamed")
            self.assertNotEqual(source_digest(root), original)

    def test_only_explicit_generated_entries_are_excluded(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "main.tf").write_text("terraform {}")
            original = source_digest(root)
            (root / "terraform.tfstate").write_text("protected state")
            (root / ".terraform").mkdir()
            (root / ".terraform" / "provider").write_bytes(b"cache")
            (root / ".docker-conformance").mkdir()
            (root / ".docker-conformance" / "journal.json").write_text("protected")
            self.assertEqual(source_digest(root), original)
            (root / "nested").mkdir()
            (root / "nested" / "terraform.tfstate").write_text(
                "source, not root runtime state"
            )
            self.assertNotEqual(source_digest(root), original)

    @unittest.skipUnless(
        sys.platform.startswith("linux"), "Linux source symlink semantics"
    )
    def test_source_file_and_directory_symlinks_rejected(self):
        for directory in (False, True):
            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / "source"
                source.mkdir() if directory else source.write_text("helper")
                (root / "link").symlink_to(source, target_is_directory=directory)
                with self.assertRaises(Rejected):
                    source_digest(root)


@unittest.skipUnless(
    sys.platform.startswith("linux"),
    "Linux process groups, pipes and private permissions",
)
class CommandTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / ".docker-conformance").mkdir(mode=0o700)
        self.commands = Commands.__new__(Commands)
        self.commands.workspace = self.root
        self.commands.env = {"PATH": os.defpath}

    def logs(self, suffix):
        return list(
            (self.root / ".docker-conformance" / "commands").glob("*." + suffix)
        )

    def test_nonzero_exit_keeps_both_streams_without_exposing_canary(self):
        code = "import sys; print('stdout-canary'); print('stderr-canary', file=sys.stderr); sys.exit(7)"
        with self.assertRaisesRegex(Rejected, "^test_apply_exit_7$"):
            self.commands.run([sys.executable, "-c", code], "test_apply")
        self.assertEqual(self.logs("stdout")[0].read_bytes(), b"stdout-canary\n")
        self.assertEqual(self.logs("stderr")[0].read_bytes(), b"stderr-canary\n")
        for log in self.logs("stdout") + self.logs("stderr"):
            self.assertEqual(stat.S_IMODE(log.stat().st_mode), 0o600)

    def test_stream_limit_retains_bounded_prefix_and_fails_uncertain(self):
        self.commands.OUTPUT_LIMIT = 1024
        code = "import os; os.write(1, b'x' * 65536)"
        with self.assertRaisesRegex(
            Rejected, "^test_output_output_limit_outcome_uncertain$"
        ):
            self.commands.run([sys.executable, "-c", code], "test_output")
        self.assertEqual(self.logs("stdout")[0].stat().st_size, 1024)

    def test_timeout_retains_flushed_diagnostic(self):
        code = "import os, time; os.write(2, b'failure-context'); time.sleep(10)"
        with self.assertRaisesRegex(
            Rejected, "^test_timeout_timeout_outcome_uncertain$"
        ):
            self.commands.run([sys.executable, "-c", code], "test_timeout", timeout=0.5)
        self.assertEqual(self.logs("stderr")[0].read_bytes(), b"failure-context")

    def test_both_large_streams_are_drained_without_deadlock(self):
        code = "import os; os.write(2, b'e' * 262144); os.write(1, b'o' * 262144)"
        output = self.commands.run(
            [sys.executable, "-c", code], "test_drains", timeout=5
        )
        self.assertEqual(output, b"o" * 262144)
        self.assertEqual(self.logs("stderr")[0].stat().st_size, 262144)


if __name__ == "__main__":
    unittest.main()
