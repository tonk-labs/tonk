#!/usr/bin/env python3
"""Exercise the real installer against local release archives, without a network."""
import hashlib
import io
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

INSTALLER = Path(__file__).resolve().parents[2] / "install.sh"


class InstallerTests(unittest.TestCase):
    def install(self, arch, slug, corrupt=False):
        with tempfile.TemporaryDirectory(prefix="tonk-install-test-") as directory:
            temp = Path(directory)
            commands = temp / "commands"
            commands.mkdir()
            release = temp / "release"
            release.mkdir()
            archive = release / f"tonk-{slug}.tar.gz"
            binary = b'#!/bin/sh\nprintf "tonk fixture\\n"\n'
            with tarfile.open(archive, "w:gz") as tar:
                info = tarfile.TarInfo("tonk")
                info.size, info.mode = len(binary), 0o755
                tar.addfile(info, io.BytesIO(binary))
            digest = hashlib.sha256(archive.read_bytes()).hexdigest()
            (release / "checksums.txt").write_text(f"{digest}  {archive.name}\n")
            if corrupt:
                archive.write_bytes(b"corrupt archive")
            (commands / "uname").write_text(
                '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo "$TEST_ARCH";; esac\n')
            (commands / "curl").write_text('''#!/bin/sh
printf '%s\n' "$2" >> "$TEST_REQUESTS"
cp "$TEST_RELEASE/${2##*/}" "$4"
''')
            for command in commands.iterdir():
                command.chmod(0o755)
            destination = temp / "installed"
            requests = temp / "requests"
            env = {**os.environ, "PATH": f"{commands}:{os.environ['PATH']}",
                   "TEST_ARCH": arch, "TEST_RELEASE": str(release), "TEST_REQUESTS": str(requests),
                   "TONK_RELEASE": "fixture", "TONK_CHANNEL": "stable",
                   "TONK_INSTALL_DIR": str(destination), "TONK_UPDATE_STATE": str(temp / "state")}
            result = subprocess.run(["sh", str(INSTALLER)], env=env, text=True,
                                    capture_output=True, timeout=15)
            fetched = requests.read_text() if requests.exists() else ""
            installed = destination / "tonk"
            if corrupt or slug == "unsupported":
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(installed.exists())
                if corrupt:
                    self.assertIn("checksum mismatch", result.stderr)
                else:
                    self.assertIn("unsupported Linux architecture", result.stderr)
                    self.assertEqual(fetched, "")
            else:
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f"/fixture/tonk-{slug}.tar.gz", fetched)
                self.assertEqual(installed.read_bytes(), binary)
                self.assertTrue(os.access(installed, os.X_OK))
                self.assertIn("checksum verified", result.stderr)
                self.assertIn("tonk fixture", result.stdout)

    def test_linux_architectures_select_matching_archive(self):
        for arch, slug in [("x86_64", "linux-x86_64"), ("amd64", "linux-x86_64"),
                           ("aarch64", "linux-arm64"), ("arm64", "linux-arm64")]:
            with self.subTest(arch=arch):
                self.install(arch, slug)

    def test_arm64_checksum_mismatch_never_installs(self):
        self.install("aarch64", "linux-arm64", corrupt=True)

    def test_32_bit_arm_is_rejected_before_download(self):
        self.install("armv7l", "unsupported")


if __name__ == "__main__":
    unittest.main()
