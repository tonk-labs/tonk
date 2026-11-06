#!/usr/bin/env python3
"""Regression tests for the CLI artifact guard (Python, cc, npm and platform tools)."""
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
from unittest import mock
import tempfile
import tarfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "check-cli.py"
spec = importlib.util.spec_from_file_location("cli_smoke", SCRIPT)
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.temp = Path(self.directory.name)
        self.source = self.temp / "main.c"
        self.source.write_text('#include <stdio.h>\nint main(void) { puts("tonk smoke"); return 0; }\n')

    def build(self, *flags):
        binary = self.temp / "tonk"
        subprocess.run(["/usr/bin/cc" if sys.platform == "darwin" else "cc", str(self.source), "-o", str(binary), *flags], check=True)
        return binary

    @unittest.skipUnless(sys.platform == "linux", "Linux ELF guard")
    def test_static_executable_is_accepted(self):
        smoke.check_static(self.build("-static"))

    @unittest.skipUnless(sys.platform == "linux", "Linux ELF guard")
    def test_dynamic_executable_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "must be static"):
            smoke.check_static(self.build())

    @unittest.skipUnless(sys.platform == "linux", "Linux ELF guard")
    def test_original_nix_interpreter_failure_is_rejected(self):
        binary = self.build("-Wl,--dynamic-linker=/nix/store/missing/lib/ld-linux-x86-64.so.2")
        with self.assertRaisesRegex(RuntimeError, "must be static"):
            smoke.check_static(binary)

    @unittest.skipUnless(sys.platform == "linux", "Linux ELF guard")
    def test_non_elf_is_rejected(self):
        with self.assertRaises(subprocess.CalledProcessError):
            smoke.check_static(self.source)

    def test_real_offline_npm_package_and_launcher(self):
        binary = self.build(*(["-static"] if sys.platform == "linux" else []))
        env = {**os.environ, "HOME": str(self.temp), "npm_config_cache": str(self.temp / "cache")}
        smoke.check_npm(binary, SCRIPT.parent.parent, self.temp, env, "tonk smoke",
                        "linux-x64" if sys.platform == "linux" else "darwin-arm64")

    def packed_manifest(self, filename, manifest):
        archive = self.temp / filename
        data = json.dumps(manifest).encode()
        with tarfile.open(archive, "w:gz") as package:
            entry = tarfile.TarInfo("package/package.json")
            entry.size = len(data)
            package.addfile(entry, io.BytesIO(data))
        return archive

    def test_packed_wrapper_requires_platform_dependency_and_selectors(self):
        for target in ["linux-x64", "darwin-arm64"]:
            target_os, cpu = target.split("-")
            manifest = {"name": f"@tonk/cli-{target}", "version": "1.2.3",
                        "os": [target_os], "cpu": [cpu]}
            platform = self.packed_manifest("platform.tgz", manifest)
            for deps in [{}, {manifest["name"]: "1.2.2"}, {manifest["name"]: "^1.2.3"}]:
                wrapper = self.packed_manifest("wrapper.tgz", {"optionalDependencies": deps})
                with self.assertRaisesRegex(RuntimeError, "exact platform optional dependency"):
                    smoke.check_packed_dependencies(platform, wrapper, target)
            wrapper = self.packed_manifest("wrapper.tgz", {
                "optionalDependencies": {manifest["name"]: "1.2.3"}})
            smoke.check_packed_dependencies(platform, wrapper, target)
            for field in ["name", "os", "cpu"]:
                bad = {**manifest, field: "wrong"}
                platform = self.packed_manifest("platform.tgz", bad)
                with self.assertRaises(RuntimeError):
                    smoke.check_packed_dependencies(platform, wrapper, target)

    @unittest.skipUnless(sys.platform == "darwin", "native macOS guard")
    def test_native_macos_executable_is_accepted(self):
        smoke.check_macos(self.build())

    @unittest.skipUnless(sys.platform == "darwin", "native macOS signature guard")
    def test_macos_guard_rejects_tampered_signature(self):
        binary = self.build()
        data = binary.read_bytes()
        self.assertIn(b"tonk smoke", data)
        binary.write_bytes(data.replace(b"tonk smoke", b"tonk broke", 1))
        with self.assertRaises(subprocess.CalledProcessError):
            smoke.check_macos(binary)

    def macos_output(self, *args, **kwargs):
        if args[0] == "lipo":
            return "arm64"
        if args[:2] == ("otool", "-hv"):
            return "MH_MAGIC_64 ARM64 EXECUTE"
        if args[:2] == ("otool", "-L"):
            return "tonk:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)"
        return ""

    def test_macos_guard_rejects_wrong_architecture_and_nix_dependency(self):
        for bad_command, output in [("lipo", "x86_64"), ("-L", "tonk:\n /nix/store/lib.dylib (compatibility version 1.0.0)"), ("-l", "path /nix/store/lib")]:
            def fake(*args, **kwargs):
                return output if bad_command in args else self.macos_output(*args, **kwargs)
            with mock.patch.object(smoke, "run", side_effect=fake):
                with self.assertRaises(RuntimeError):
                    smoke.check_macos(Path("tonk"))

    def test_pr_signature_integrity_does_not_require_notarization(self):
        with mock.patch.object(smoke, "run", side_effect=self.macos_output) as run:
            smoke.check_macos(Path("tonk"))
            calls = [call.args for call in run.call_args_list]
            self.assertIn(("codesign", "--verify", "--strict", "tonk"), calls)
            self.assertFalse(any("--check-notarization" in call for call in calls))

    def test_release_requires_developer_id_and_notarization(self):
        with mock.patch.object(smoke, "run", side_effect=self.macos_output) as run:
            with mock.patch.object(smoke.subprocess, "run", return_value=mock.Mock(stderr="Signature=adhoc")):
                with self.assertRaisesRegex(RuntimeError, "Developer ID"):
                    smoke.check_macos(Path("tonk"), True)
            with mock.patch.object(smoke.subprocess, "run", return_value=mock.Mock(stderr="Authority=Developer ID Application: Test")):
                smoke.check_macos(Path("tonk"), True)
            self.assertTrue(any("--check-notarization" in call.args for call in run.call_args_list))


if __name__ == "__main__":
    unittest.main()
