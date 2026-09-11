"""Mock guest execution; these tests neither provision a VM nor run GitHub jobs."""

import unittest

from guest_fixture import JIT_CANARY, TEMPLATE, GuestFixture


class GuestBootstrapTests(unittest.TestCase):
    def setUp(self):
        self.guest = GuestFixture()
        self.addCleanup(self.guest.close)

    def assert_no_secret_output(self, result):
        self.assertNotIn(JIT_CANARY, result.stdout + result.stderr + self.guest.trace())

    def assert_stopped_before_listener(self, result):
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("listener-env-ok", self.guest.trace())
        self.assertTrue((self.guest.root / "var/lib/shaula/started").is_dir())
        self.assert_no_secret_output(result)

    def test_success_consumes_jit_after_hook_and_seed_cleanup_exactly_once(self):
        result = self.guest.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.guest.trace()
        self.assertLess(trace.index("pre-start"), trace.index("udevadm"))
        self.assertLess(trace.index("chmod <0600>"), trace.index("umount"))
        self.assertLess(trace.index("chown <root:root>"), trace.index("umount"))
        self.assertLess(trace.index("umount"), trace.index("runuser"))
        self.assertIn("runuser <-u> <runner> <--> ", trace)
        self.assertEqual(trace.count("listener-env-ok"), 1)
        self.assert_no_secret_output(result)

        repeated = self.guest.run()
        self.assertNotEqual(repeated.returncode, 0)
        self.assertEqual(self.guest.trace().count("listener-env-ok"), 1)
        self.assertEqual(self.guest.trace().count("pre-start"), 1)

    def test_workdir_precreated_for_runner_before_listener(self):
        # The scale-set JIT config pins the ARC-style absolute "/_work"
        # workFolder; the guest must hand the runner user a writable one.
        result = self.guest.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.guest.trace()
        self.assertIn("chown <runner:runner>", trace)
        self.assertLess(trace.index("chown <runner:runner>"), trace.index("runuser"))
        self.assertTrue((self.guest.root / "_work").is_dir())

    def test_pre_start_failure_prevents_registration_and_retry(self):
        self.guest.write("var/lib/shaula/pre-start", "exit 17\n")
        self.assert_stopped_before_listener(self.guest.run())
        self.assertTrue((self.guest.root / "var/lib/shaula/jit-config").is_file())
        self.guest.write("var/lib/shaula/pre-start", "exit 0\n")
        self.assert_stopped_before_listener(self.guest.run())

    def test_findmnt_failure_cannot_be_hidden_by_shell_process_substitution(self):
        self.assert_stopped_before_listener(self.guest.run(FINDMNT_STATUS="2"))
        self.assertTrue((self.guest.root / "var/lib/shaula/jit-config").is_file())

    def test_already_unmounted_seed_is_accepted(self):
        result = self.guest.run(FINDMNT_STATUS="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertNotIn("umount", self.guest.trace())
        self.assertIn("listener-env-ok", self.guest.trace())

    def test_unmount_failure_blocks_listener(self):
        self.assert_stopped_before_listener(self.guest.run(UMOUNT_STATUS="32"))

    def test_provider_seed_label_and_udev_permissions_match(self):
        result = self.guest.run()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("blkid <-L> <CIDATA>", self.guest.trace())
        cloud = (TEMPLATE / "user-data.tftpl").read_text(encoding="utf-8")
        self.assertIn(r"ENV{ID_FS_LABEL}==\"CIDATA\"", cloud)
        self.assertIn(r"OWNER=\"root\", GROUP=\"root\", MODE=\"0600\"", cloud)

    def test_empty_jit_blocks_listener(self):
        self.guest.write("var/lib/shaula/jit-config", "")
        self.assert_stopped_before_listener(self.guest.run())

    def test_listener_failure_does_not_register_again(self):
        result = self.guest.run(LISTENER_STATUS="19")
        self.assertEqual(result.returncode, 19)
        self.assertEqual(self.guest.trace().count("listener-env-ok"), 1)
        self.assertNotEqual(self.guest.run().returncode, 0)
        self.assertEqual(self.guest.trace().count("listener-env-ok"), 1)

    def test_unit_defers_launch_until_cloud_final_and_has_no_reboot_install(self):
        unit = (TEMPLATE / "runner-service.tftpl").read_text(encoding="utf-8")
        cloud = (TEMPLATE / "user-data.tftpl").read_text(encoding="utf-8")
        self.assertIn("After=network-online.target cloud-final.service", unit)
        self.assertIn("ConditionPathExists=!/var/lib/shaula/started", unit)
        self.assertIn("Restart=no", unit)
        self.assertNotIn("[Install]", unit)
        self.assertIn(
            '["systemctl", "start", "--no-block", "shaula-runner.service"]', cloud
        )
        self.assertNotIn('"enable"', cloud)


if __name__ == "__main__":
    unittest.main()
