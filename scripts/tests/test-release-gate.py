#!/usr/bin/env python3
"""Negative-path release policy tests. All records here are synthetic."""
from copy import deepcopy
from datetime import datetime, timedelta, timezone
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_gate as gate

ROOT = Path(__file__).resolve().parents[2]
NOW = datetime(2026, 10, 13, 12, tzinfo=timezone.utc)


def fixture():
    policy = gate.load(ROOT / "release/flows.json")
    context = {"schema": 1, "repository": "tonk-labs/tonk", "source_sha": "a" * 40,
               "controller_sha": "b" * 40, "policy_digest": gate.digest(policy),
               "run_id": 123, "run_attempt": 1, "version": "1.2.3-rc.1", "channel": "staging"}
    candidate = {"context": context, "artifacts": {name: "c" * 64 for name in policy["artifacts"]}}
    names = sorted({test for flow in policy["flows"] for test in flow["tests"]})
    shards = []
    for n in range(3):
        subset = names[n::3]
        shards.append({"id": str(n + 1), "context": deepcopy(context),
                       "artifacts": {k: candidate["artifacts"][k] for k in ("linux-cli", "web")},
                       "inventory": subset,
                       "tests": [{"id": name, "status": "passed", "attempts": 1} for name in subset]})
    evidence = {"context": deepcopy(context), "candidate_digest": gate.digest(candidate),
                "checks": [{"id": check, "status": "passed"} for check in policy["checks"]],
                "shards": shards, "reviews": []}
    for group in policy["manual_groups"]:
        evidence["reviews"].append({"id": group, "candidate_digest": gate.digest(candidate),
                                    "status": "passed", "actor": "synthetic-reviewer",
                                    "verified_at": NOW.isoformat(),
                                    "evidence_url": "https://example.test/synthetic-evidence"})
    return candidate, policy, evidence


class GateTests(unittest.TestCase):
    def setUp(self):
        self.candidate, self.policy, self.evidence = fixture()

    def result(self):
        return gate.evaluate(self.candidate, self.policy, self.evidence, now=NOW)

    def blocked(self, reason):
        result = self.result()
        self.assertFalse(result["ready"])
        self.assertIn(reason, "\n".join(result["blockers"]))

    def test_policy_test_names_and_journey_ids_exist(self):
        source = (ROOT / "rust/tonk-ui/src/account_flow.rs").read_text()
        journeys = (ROOT / "docs/storybook/journey-catalog.md").read_text()
        for flow in self.policy["flows"]:
            for test in flow["tests"]:
                self.assertTrue(test.startswith("tonk-ui::account_flow::tests::"))
                self.assertIn("fn " + test.rsplit("::", 1)[1] + "(", source)
            for journey in flow["journeys"]:
                self.assertIn("`" + journey + "`", journeys)

    def test_complete_synthetic_candidate(self):
        self.assertTrue(self.result()["ready"])
        self.assertEqual(len(self.result()["flows"]), 10)

    def test_different_identity(self):
        for field, value in (("source_sha", "d" * 40), ("controller_sha", "d" * 40),
                             ("run_id", 124), ("run_attempt", 2), ("policy_digest", "e" * 64)):
            with self.subTest(field=field):
                changed = deepcopy(self.evidence)
                changed["context"][field] = value
                self.assertFalse(gate.evaluate(self.candidate, self.policy, changed, now=NOW)["ready"])

    def test_artifact_mismatch(self):
        self.candidate["artifacts"]["linux-cli"] = "f" * 64
        self.blocked("artifact identity mismatch")

    def test_tests_ran_other_binaries(self):
        self.evidence["shards"][0]["artifacts"]["web"] = "f" * 64
        self.blocked("different artifacts")

    def test_missing_artifact(self):
        del self.candidate["artifacts"]["macos-cli"]
        self.blocked("missing or unknown artifact")

    def test_chatgpt_contracts_are_required_without_manual_chatgpt_group(self):
        self.assertIn("chatgpt", self.policy["checks"])
        self.assertNotIn("chatgpt", self.policy["manual_groups"])
        for status in ("failed", "skipped", "cancelled", "missing"):
            with self.subTest(status=status):
                self.evidence["checks"] = [c for c in self.evidence["checks"] if c["id"] != "chatgpt"]
                if status != "missing":
                    self.evidence["checks"].append({"id": "chatgpt", "status": status})
                self.blocked("chatgpt:")
                row = next(c for c in self.result()["checks"] if c["id"] == "chatgpt")
                self.assertNotEqual(row["status"], "passed")

    def test_missing_check(self):
        self.evidence["checks"].pop()
        self.blocked("e2e: missing")

    def test_all_non_passed_conclusions_block(self):
        for status in ("failed", "flaky", "skipped", "cancelled", "pending", "neutral", None):
            with self.subTest(status=status):
                self.evidence["checks"][0]["status"] = status
                self.blocked("tests:")

    def test_missing_shard(self):
        self.evidence["shards"].pop()
        self.blocked("shard 3: missing")

    def test_missing_mapped_test_even_if_not_discovered(self):
        shard = self.evidence["shards"][0]
        name = shard["inventory"].pop()
        shard["tests"] = [test for test in shard["tests"] if test["id"] != name]
        self.blocked("required mapped test missing")

    def test_unmapped_discovered_test_cannot_fail_silently(self):
        self.evidence["shards"][0]["inventory"].append("another::test")
        self.blocked("another::test: missing")

    def test_duplicate_and_empty_inventory(self):
        self.evidence["shards"][0]["inventory"] = []
        self.blocked("empty inventory")

    def test_duplicate_shard(self):
        self.evidence["shards"].append(deepcopy(self.evidence["shards"][0]))
        self.blocked("duplicate shard")

    def test_duplicate_test(self):
        shard = self.evidence["shards"][0]
        shard["tests"].append(deepcopy(shard["tests"][0]))
        self.blocked("duplicate test")

    def test_flaky_pass_cannot_hide_retry(self):
        self.evidence["shards"][0]["tests"][0]["attempts"] = 2
        self.blocked("attempts=2")

    def test_mixed_run_attempts(self):
        self.evidence["shards"][0]["context"]["run_attempt"] = 2
        self.blocked("shard candidate identity mismatch")

    def test_missing_review(self):
        self.evidence["reviews"] = []
        self.blocked("Manual verification accounts: missing")

    def test_expired_and_future_review(self):
        for hours in (-25, 1):
            self.evidence["reviews"][0]["verified_at"] = (NOW + timedelta(hours=hours)).isoformat()
            self.blocked("Manual verification accounts: blocked")

    def test_url_cannot_execute_code(self):
        self.evidence["reviews"][0]["evidence_url"] = "javascript:alert(1)"
        self.blocked("Manual verification accounts: blocked")

    def test_unknown_review_group(self):
        self.evidence["reviews"][0]["id"] = "made-up"
        self.blocked("unknown review group")

    def stable(self):
        context = self.candidate["context"]
        context.update(channel="stable", version="1.2.3", staging_candidate_digest="e" * 64)
        self.evidence["context"] = deepcopy(context)
        self.evidence["candidate_digest"] = gate.digest(self.candidate)
        self.evidence["staging"] = {"source_sha": context["source_sha"], "version": "1.2.3-rc.2",
                                    "candidate_digest": "e" * 64, "current": True,
                                    "deliveries": dict(web="passed", github="passed", npm="passed"),
                                    "deployment_id": "synthetic-deployment"}
        for shard in self.evidence["shards"]:
            shard["context"] = deepcopy(context)
        for review in self.evidence["reviews"]:
            review["candidate_digest"] = gate.digest(self.candidate)
            review["staging_digest"] = gate.digest(self.evidence["staging"])

    def test_stable_requires_complete_same_source_staging(self):
        self.stable()
        self.assertTrue(self.result()["ready"])
        self.evidence["staging"]["deliveries"]["npm"] = "failed"
        self.blocked("failed staging publication")

    def test_superseded_staging_blocks(self):
        self.stable()
        self.evidence["staging"]["current"] = False
        self.blocked("superseded")

    def test_review_of_different_deployment_blocks(self):
        self.stable()
        self.evidence["staging"]["deployment_id"] = "replacement"
        self.blocked("Manual verification accounts: blocked")

    def test_json_duplicate_keys_rejected(self):
        with tempfile.TemporaryDirectory() as folder:
            path = Path(folder) / "record.json"
            path.write_text('{"status":"failed","status":"passed"}')
            with self.assertRaisesRegex(gate.InvalidEvidence, "duplicate JSON key"):
                gate.load(path)


if __name__ == "__main__":
    unittest.main()
