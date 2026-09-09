import importlib.util
import json
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "prepare", Path(__file__).with_name("prepare.py")
)
generator = importlib.util.module_from_spec(spec)
spec.loader.exec_module(generator)


class TemplateTests(unittest.TestCase):
    def test_generates_separate_v2_sources_and_keeps_resource_shape(self):
        with tempfile.TemporaryDirectory() as directory:
            for platform in ("docker", "kubernetes"):
                output = Path(directory) / platform
                # Synthetic pin is only a unit-test fixture, never a shipping artifact.
                generator.prepare(
                    platform, "fixture.invalid/runner:v2@sha256:" + "a1" * 32, output
                )
                profile = (output / "profile.yaml").read_text()
                main = (output / "main.tf").read_text()
                self.assertIn("input_contract_version: 2", profile)
                self.assertIn("setup_info_contract: shaula.setup-info/v1", profile)
                self.assertIn("setup_info = any", main)
                self.assertIn(
                    "/shaula/setup_info.json"
                    if platform == "docker"
                    else "cp /in/setup_info.json",
                    main,
                )
                self.assertEqual(
                    main.count('resource "'), 1 if platform == "docker" else 2
                )
                parameters = json.loads(
                    (output / "schemas/parameters.schema.json").read_text()
                )
                self.assertEqual(
                    parameters["properties"]["runner_image"]["enum"],
                    ["fixture.invalid/runner:v2"],
                )

    def test_refuses_old_image_and_existing_output(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            with self.assertRaises(ValueError):
                generator.prepare(
                    "docker", "fixture.invalid/runner:v2@sha256:" + "a1" * 32, output
                )
            old = "localhost:5001/shaula-runner:2.337.0-bootstrap-v1@sha256:eb9fa6d0a3b8688f0f1daba9989f9323ad02b02c6d1cd36d49c2fddcb7fb6201"
            with self.assertRaises(ValueError):
                generator.prepare("docker", old, output / "new")


if __name__ == "__main__":
    unittest.main()
