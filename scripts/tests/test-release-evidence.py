#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_artifacts as artifacts
import release_evidence as evidence
import release_gate as gate
import release_report as report
import release_stamp as stamp

spec = importlib.util.spec_from_file_location("gate_tests", Path(__file__).with_name("test-release-gate.py"))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class EvidenceTests(unittest.TestCase):
    def test_maintain_role_uses_role_name(self):
        api = evidence.GitHub("tonk-labs/tonk")
        with patch.object(api, "get", return_value={"permission": "write", "role_name": "maintain"}):
            api.authorized_reviewer("maintainer")

    def test_inventory_only_selected_cases(self):
        data = {"rust-suites": {"test": {"status": "listed", "binary-id": "package::binary", "testcases": {
            "selected": {"ignored": False, "filter-match": {"status": "matches"}},
            "other-shard": {"ignored": False, "filter-match": {"status": "mismatch"}},
        }}}}
        self.assertEqual(evidence.inventory(data), ["package::binary::selected"])

    def test_missing_inventory_is_never_green(self):
        with self.assertRaisesRegex(gate.InvalidEvidence, "empty"):
            evidence.inventory({"rust-suites": {}})

    def xml(self, child=""):
        return (f'<testsuites><testsuite name="tonk-ui" tests="1"><testcase classname="tonk-ui" '
                f'name="test">{child}</testcase></testsuite></testsuites>').encode()

    def test_junit_classifies_retries_and_drops_output(self):
        result = evidence.junit(self.xml('<flakyFailure>private callback</flakyFailure><system-out>private data</system-out>'))
        self.assertEqual(result, [{"id": "tonk-ui::test", "status": "flaky", "attempts": 2}])
        self.assertNotIn("private", json.dumps(result))

    def test_junit_fail_skip_pass(self):
        for child, status in (("<failure/>", "failed"), ("<error/>", "failed"),
                              ("<skipped/>", "blocked"), ("", "passed")):
            self.assertEqual(evidence.junit(self.xml(child))[0]["status"], status)

    def test_entities_and_unknown_results_rejected(self):
        for data in (b'<!DOCTYPE x><testsuites/>', self.xml("<newUnknownStatus/>"), b'<testsuites/>'):
            with self.assertRaises(gate.InvalidEvidence):
                evidence.junit(data)

    def test_suite_count_mismatch(self):
        with self.assertRaisesRegex(gate.InvalidEvidence, "incomplete"):
            evidence.junit(self.xml().replace(b'tests="1"', b'tests="2"'))

    def test_html_and_markdown_escape_untrusted_labels(self):
        candidate, policy, data = fixtures.fixture()
        result = gate.evaluate(candidate, policy, data, now=fixtures.NOW)
        result["flows"][0]["title"] = '<script>alert("x")</script> | injected\nrow'
        rendered = report.render_html(result)
        self.assertNotIn("<script>", rendered)
        self.assertIn("&lt;script&gt;", rendered)
        self.assertIn("&#124;", report.markdown(result))
        self.assertIn("default-src 'none'", rendered)

    def test_artifact_tree_sensitive_to_names_and_bytes(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            (path / "a").write_bytes(b"hello")
            first = artifacts.tree_digest(path)
            (path / "a").rename(path / "b")
            self.assertNotEqual(first, artifacts.tree_digest(path))
            (path / "link").symlink_to(path / "b")
            with self.assertRaisesRegex(gate.InvalidEvidence, "symlink"):
                artifacts.tree_digest(path)

    def test_stamp_only_workspace_packages(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder)
            (path / "Cargo.toml").write_text('[workspace.package]\nversion = "1.0.0"\n[other]\nversion = "1.0.0"\n')
            (path / "Cargo.lock").write_text('version = 4\n\n[[package]]\nname = "local"\nversion = "1.0.0"\n\n[[package]]\nname = "external"\nversion = "1.0.0"\nsource = "registry"\n')
            stamp.stamp(path, "1.2.3-rc.1")
            self.assertIn('[other]\nversion = "1.0.0"', (path / "Cargo.toml").read_text())
            self.assertIn('name = "external"\nversion = "1.0.0"', (path / "Cargo.lock").read_text())
            with self.assertRaises(gate.InvalidEvidence):
                stamp.stamp(path, "1.2.3;echo bad")

    def test_remote_workflow_origin_checks(self):
        api = evidence.GitHub("tonk-labs/tonk")
        run = {"repository": {"full_name": "tonk-labs/tonk"}, "path": ".github/workflows/release-readiness.yml",
               "head_branch": "main", "event": "workflow_dispatch", "status": "completed"}
        with patch.object(api, "get", return_value=run):
            self.assertEqual(api.run(123, "release-readiness.yml"), run)
            for key, value in (("head_branch", "other"), ("event", "pull_request"), ("status", "in_progress")):
                changed = {**run, key: value}
                with patch.object(api, "get", return_value=changed):
                    with self.assertRaises(gate.InvalidEvidence):
                        api.run(123, "release-readiness.yml")

    def test_nonmaintainer_cannot_record_review(self):
        api = evidence.GitHub("tonk-labs/tonk")
        with patch.object(api, "get", return_value={"permission": "write"}):
            with self.assertRaisesRegex(gate.InvalidEvidence, "maintain"):
                api.authorized_reviewer("writer")


if __name__ == "__main__":
    unittest.main()
