"""No GitHub credentials or network: validate the JIT handoff contract."""

import base64
import importlib.machinery
import importlib.util
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

SHIM_PATH = Path(__file__).with_name("bootstrap-shim")
loader = importlib.machinery.SourceFileLoader("bootstrap_shim", str(SHIM_PATH))
spec = importlib.util.spec_from_loader(loader.name, loader)
shim = importlib.util.module_from_spec(spec)
loader.exec_module(shim)


def encoded(value):
    return base64.b64encode(json.dumps(value).encode("utf-8"))


def fixture():
    # Deliberately unusable credentials; the fake Listener checks transport.
    return {
        ".runner": encoded({"ephemeral": True, "agentName": "test-only"}).decode(),
        ".credentials": encoded({"test": "not-a-credential"}).decode(),
        ".credentials_rsaparams": encoded({"test": "not-a-key"}).decode(),
    }


VALID_JIT = encoded(fixture())


class EncodingTests(unittest.TestCase):
    def test_preserves_exact_valid_encoded_value(self):
        self.assertEqual(shim.validate_jit(VALID_JIT), VALID_JIT.decode("ascii"))

    def test_rejects_malformed_envelopes(self):
        invalid = [
            b"",
            b"not base64!",
            VALID_JIT + b"\n",
            VALID_JIT + b"=",
            base64.b64encode(b"not json"),
            encoded([]),
            encoded({}),
            encoded({".runner": 7}),
            b"A" * (shim.MAX_JIT_BYTES + 1),
        ]
        for raw in invalid:
            with self.subTest(size=len(raw)), self.assertRaises(ValueError):
                shim.validate_jit(raw)

    def test_rejects_path_injection_and_bad_configuration_values(self):
        cases = []
        for name in ("../escape", "/absolute", "nested/file", ".env"):
            value = fixture()
            value[name] = encoded({"test": True}).decode()
            cases.append(value)
        for bad in (None, 1, "", "bad!", encoded([]).decode()):
            value = fixture()
            value[".credentials"] = bad
            cases.append(value)
        for value in cases:
            with self.subTest(), self.assertRaises((ValueError, TypeError)):
                shim.validate_jit(encoded(value))

    def test_rejects_duplicate_outer_members(self):
        raw = json.dumps(fixture())
        duplicate = raw[:-1] + ', ".runner": "e30="}'
        with self.assertRaises(ValueError):
            shim.validate_jit(base64.b64encode(duplicate.encode()))


@unittest.skipUnless(sys.platform.startswith("linux"), "Linux file/exec semantics")
class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.jit_path = self.root / "jit_config"
        self.jit_path.write_bytes(VALID_JIT)

    def consume(self):
        with mock.patch.object(shim, "JIT_PATH", self.jit_path):
            return shim.consume_jit()

    def test_consumes_upload_once(self):
        with mock.patch.object(shim.os, "open", wraps=os.open) as opened:
            self.assertEqual(self.consume(), VALID_JIT.decode())
        self.assertEqual(opened.call_count, 1)
        self.assertFalse(self.jit_path.exists())
        with self.assertRaises(FileNotFoundError):
            self.consume()

    def test_malformed_consumed_upload_is_removed(self):
        self.jit_path.write_bytes(b"malformed-test-marker")
        with self.assertRaises(ValueError):
            self.consume()
        self.assertFalse(self.jit_path.exists())

    def test_rejects_symlink_and_does_not_read_its_target(self):
        target = self.root / "target"
        self.jit_path.rename(target)
        self.jit_path.symlink_to(target)
        with self.assertRaises(OSError):
            self.consume()
        self.assertEqual(target.read_bytes(), VALID_JIT)

    def test_rejects_fifo_without_blocking(self):
        self.jit_path.unlink()
        os.mkfifo(self.jit_path)
        with self.assertRaises(ValueError):
            self.consume()

    def launch(self, listener):
        # Paths are test-only Python module bindings, not production CLI
        # options or environment overrides accepted by the shipped shim.
        harness = (
            "import runpy,sys; "
            "m=runpy.run_path(sys.argv[1]); "
            "g=m['main'].__globals__; "
            "g['JIT_PATH']=g['Path'](sys.argv[2]); "
            "g['RUNNER_ROOT']=g['Path'](sys.argv[3]); "
            "g['LISTENER']=sys.argv[4]; "
            "sys.argv=[sys.argv[1]]; "
            "sys.exit(g['main']())"
        )
        environment = os.environ.copy()
        environment["ACTIONS_RUNNER_INPUT_TOKEN"] = "must-not-reach-listener"
        environment["actions_runner_input_URL"] = "must-not-reach-listener"
        environment["ACTIONS_RUNNER_INPUT_JITCONFIG"] = "must-be-replaced"
        return subprocess.run(
            [
                sys.executable,
                "-I",
                "-c",
                harness,
                str(SHIM_PATH),
                str(self.jit_path),
                str(self.root),
                str(listener),
            ],
            env=environment,
            capture_output=True,
            timeout=10,
            check=False,
        )

    def test_fake_listener_observes_handoff_and_propagates_exit_status(self):
        listener = self.root / "fake-listener"
        listener.write_text(
            f"#!{sys.executable}\n"
            "import os,sys,pathlib\n"
            "assert sys.argv[1:] == ['run']\n"
            "assert not pathlib.Path('jit_config').exists()\n"
            "keys=[k for k in os.environ if k.upper().startswith('ACTIONS_RUNNER_INPUT_')]\n"
            "assert keys == ['ACTIONS_RUNNER_INPUT_JITCONFIG']\n"
            f"assert os.environ.pop(keys[0]) == {VALID_JIT.decode()!r}\n"
            "assert not any(k.startswith('ACTIONS_RUNNER_INPUT_') for k in os.environ)\n"
            "assert os.umask(0o077) == 0o077\n"
            "pathlib.Path('observed').write_text('handoff verified')\n"
            "sys.exit(23)\n"
        )
        listener.chmod(0o755)
        result = self.launch(listener)
        self.assertEqual(result.returncode, 23, result.stderr.decode())
        self.assertEqual((self.root / "observed").read_text(), "handoff verified")
        self.assertEqual(result.stdout, b"")
        self.assertEqual(result.stderr, b"")
        # A second launch cannot reuse an already consumed registration.
        result = self.launch(listener)
        self.assertEqual(result.returncode, 78)

    def test_invalid_upload_never_launches_listener_or_echoes_input(self):
        listener = self.root / "unexpected-listener"
        listener.write_text(
            f"#!{sys.executable}\n"
            "from pathlib import Path\n"
            "Path('unexpected-execution').touch()\n"
        )
        listener.chmod(0o755)
        for value in (None, b"", b"malformed-test-marker", b"A" * 65537):
            if self.jit_path.exists():
                self.jit_path.unlink()
            if value is not None:
                self.jit_path.write_bytes(value)
            with self.subTest(size=None if value is None else len(value)):
                result = self.launch(listener)
                self.assertEqual(result.returncode, 78)
                self.assertEqual(result.stdout, b"")
                self.assertEqual(
                    result.stderr, b"bootstrap-shim: JIT bootstrap failed\n"
                )
                self.assertFalse((self.root / "unexpected-execution").exists())

    def test_unlink_failure_prevents_exec(self):
        with (
            mock.patch.object(shim, "JIT_PATH", self.jit_path),
            mock.patch.object(shim.Path, "unlink", side_effect=PermissionError),
            mock.patch.object(shim.os, "execve") as execute,
            mock.patch.object(shim.sys, "argv", ["bootstrap-shim"]),
            mock.patch.object(shim.sys, "stderr"),
        ):
            self.assertEqual(shim.main(), 78)
        execute.assert_not_called()


if __name__ == "__main__":
    unittest.main()
