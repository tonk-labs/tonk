"""The signed Desktop builds bake the same PostHog key as the CLI and web."""
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


class PostHogKeyTest(unittest.TestCase):
    def test_signed_desktop_builds_use_the_flake_key(self):
        flake = re.search(r'posthogKey = "([^"]+)";', (ROOT / "flake.nix").read_text())
        self.assertIsNotNone(flake)
        for workflow in ("desktop.yml", "desktop-release.yml"):
            text = (ROOT / ".github/workflows" / workflow).read_text()
            keys = re.findall(r"TONK_POSTHOG_KEY: (\S+)", text)
            self.assertEqual(keys, [flake.group(1)], workflow)


if __name__ == "__main__":
    unittest.main()
