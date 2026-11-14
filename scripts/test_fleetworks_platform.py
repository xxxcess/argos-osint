"""Offline physical-host and target-selection contracts."""

import subprocess
import unittest

from fleetworks_platform import detect, format_platform, resolve_native_target


class PlatformTests(unittest.TestCase):
    def detect(self, system="Darwin", machine="x86_64", translated="0", arm64="0",
               host="x86_64-apple-darwin", rust_error=False):
        self.commands = []

        def runner(command, **kwargs):
            self.commands.append(command)
            self.assertEqual(kwargs["env"]["RUSTUP_AUTO_INSTALL"], "0")
            if command[0] == "sysctl":
                value = translated if command[-1] == "sysctl.proc_translated" else arm64
                return subprocess.CompletedProcess(command, 0, value, "")
            if rust_error:
                return subprocess.CompletedProcess(command, 1, "", "toolchain unavailable")
            return subprocess.CompletedProcess(command, 0, f"rustc 1.95.0\nhost: {host}\nrelease: 1.95.0\n", "")

        return detect(system=system, process_machine=machine, runner=runner, environment={})

    def test_intel_mac_selects_matching_native_and_linux_targets(self):
        info = self.detect()
        self.assertEqual(resolve_native_target(info), "x86_64-apple-darwin")
        self.assertEqual(format_platform(info, "docker"), "linux/amd64")
        self.assertFalse(info.rosetta)
        self.assertEqual(info.physical_architecture, "x86_64")

    def test_native_apple_silicon_selects_arm_and_reports_toolchain(self):
        info = self.detect(machine="arm64", arm64="1", host="aarch64-apple-darwin")
        self.assertEqual(resolve_native_target(info), "aarch64-apple-darwin")
        self.assertEqual(info.docker_platform, "linux/arm64")
        self.assertEqual(info.rust_release, "1.95.0")

    def test_rosetta_reports_physical_arm_but_requires_explicit_cross_target(self):
        info = self.detect(translated="1", arm64="1")
        self.assertTrue(info.rosetta)
        self.assertEqual(info.physical_architecture, "arm64")
        self.assertEqual(info.process_architecture, "x86_64")
        self.assertEqual(info.docker_platform, "linux/arm64")
        with self.assertRaisesRegex(ValueError, "Rosetta"):
            resolve_native_target(info)
        self.assertEqual(resolve_native_target(info, "x86_64-apple-darwin"), "x86_64-apple-darwin")

    def test_mismatched_toolchain_cannot_silently_choose_host_target(self):
        info = self.detect(host="aarch64-apple-darwin")
        with self.assertRaisesRegex(ValueError, "does not match physical Mac"):
            resolve_native_target(info)

    def test_linux_platform_needs_no_host_rust_or_mac_product(self):
        info = self.detect(system="Linux", machine="aarch64", rust_error=True)
        self.assertEqual(format_platform(info, "docker"), "linux/arm64")
        self.assertEqual(info.rust_host, "")
        self.assertIn("toolchain unavailable", info.rust_error)
        with self.assertRaisesRegex(ValueError, "requires a supported"):
            format_platform(info, "rust")
        self.assertEqual(self.commands, [["rustc", "-vV"]])

    def test_unsupported_hosts_and_targets_fail_explicitly(self):
        info = self.detect(system="Windows", machine="AMD64")
        with self.assertRaisesRegex(ValueError, "Unsupported Linux backend"):
            format_platform(info, "docker")
        with self.assertRaisesRegex(ValueError, "Unsupported native target"):
            resolve_native_target(self.detect(), "x86_64-pc-windows-msvc")

    def test_missing_rust_does_not_install_it(self):
        info = self.detect(rust_error=True)
        self.assertEqual(info.docker_platform, "linux/amd64")
        with self.assertRaisesRegex(ValueError, "repository-pinned toolchain"):
            resolve_native_target(info)
        self.assertTrue(all(command[0] in ("sysctl", "rustc") for command in self.commands))


if __name__ == "__main__":
    unittest.main()
