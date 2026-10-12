#!/usr/bin/env python3
"""Regression tests for the Linux release guard (Python, cc and binutils)."""
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tarfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "check-linux-cli.py"
spec = importlib.util.spec_from_file_location("linux_cli", SCRIPT)
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class LinuxArtifactTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.temp = Path(self.directory.name)
        self.source = self.temp / "main.c"
        self.source.write_text('#include <stdio.h>\nint main(void) { puts("tonk smoke"); return 0; }\n')

    def build(self, *flags):
        binary = self.temp / "tonk"
        subprocess.run(["cc", str(self.source), "-o", str(binary), *flags], check=True)
        return binary

    def test_static_executable_is_accepted(self):
        smoke.check_static(self.build("-static"))

    def test_dynamic_executable_is_rejected(self):
        with self.assertRaisesRegex(RuntimeError, "must be static"):
            smoke.check_static(self.build())

    def test_original_nix_interpreter_failure_is_rejected(self):
        binary = self.build("-Wl,--dynamic-linker=/nix/store/missing/lib/ld-linux-x86-64.so.2")
        with self.assertRaisesRegex(RuntimeError, "must be static"):
            smoke.check_static(binary)

    def test_non_elf_is_rejected(self):
        with self.assertRaises(subprocess.CalledProcessError):
            smoke.check_static(self.source)

    def test_real_offline_npm_package_and_launcher(self):
        binary = self.build("-static")
        env = {**os.environ, "HOME": str(self.temp), "npm_config_cache": str(self.temp / "cache")}
        smoke.check_npm(binary, SCRIPT.parent.parent, self.temp, env, "tonk smoke")

    def packed_manifest(self, filename, manifest):
        archive = self.temp / filename
        data = json.dumps(manifest).encode()
        with tarfile.open(archive, "w:gz") as package:
            entry = tarfile.TarInfo("package/package.json")
            entry.size = len(data)
            package.addfile(entry, io.BytesIO(data))
        return archive

    def test_packed_wrapper_requires_linux_optional_dependency(self):
        platform = self.packed_manifest("platform.tgz", {
            "name": "@tonk/cli-linux-x64", "version": "1.2.3",
        })
        wrapper = self.packed_manifest("wrapper.tgz", {"name": "@tonk/cli"})
        with self.assertRaisesRegex(RuntimeError, "exact Linux optional dependency"):
            smoke.check_packed_dependencies(platform, wrapper)

    def test_packed_wrapper_requires_exact_platform_version(self):
        platform = self.packed_manifest("platform.tgz", {
            "name": "@tonk/cli-linux-x64", "version": "1.2.3",
        })
        for version in ["1.2.2", "^1.2.3"]:
            wrapper = self.packed_manifest("wrapper.tgz", {
                "optionalDependencies": {"@tonk/cli-linux-x64": version},
            })
            with self.assertRaisesRegex(RuntimeError, "exact Linux optional dependency"):
                smoke.check_packed_dependencies(platform, wrapper)


if __name__ == "__main__":
    unittest.main()
