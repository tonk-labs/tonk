#!/usr/bin/env python3
from copy import deepcopy
import importlib.util
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_policy as policy
import release_promote as promote

spec = importlib.util.spec_from_file_location("gate_tests", Path(__file__).with_name("test-release-gate.py"))
fixtures = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixtures)


class FakeGitHub:
    def __init__(self):
        self.environments = {name: {"can_admins_bypass": False,
            "protection_rules": [{"type": "required_reviewers", "reviewers": [{"type": "User", "id": 1}]}],
            "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True}}
            for name in ("release-staging", "release-stable")}
        common = {"enforcement": "active", "target": "tag", "conditions": {"ref_name": {"include": ["refs/tags/v*"], "exclude": []}}}
        self.rules = [dict(deepcopy(common), id=1, rules=[{"type": "creation"}],
                           bypass_actors=[{"actor_id": 99, "actor_type": "Integration", "bypass_mode": "always"}]),
                      dict(deepcopy(common), id=2, rules=[{"type": "update"}, {"type": "deletion"}], bypass_actors=[])]
        self.branches = [{"name": "main", "type": "branch"}]
        self.people = [{"login": "maintainer", "permissions": {"admin": True}}]

    def get(self, path):
        if path.startswith("environments/"):
            return self.environments[path.split("/")[1]]
        return next(rule for rule in self.rules if str(rule["id"]) == path.split("/")[1])

    def pages(self, path, key):
        if path == "rulesets":
            return self.rules
        if path.startswith("environments/"):
            return self.branches
        return self.people


class PolicyTests(unittest.TestCase):
    def setUp(self):
        self.api = FakeGitHub()
        self.config = {"release_app_id": 99, "release_custodians": ["maintainer"],
                       "cutover_evidence_url": "https://github.com/example/synthetic/actions/runs/1"}

    def blocked(self, message):
        result = policy.audit(self.api, self.config)
        self.assertFalse(result["ready"])
        self.assertIn(message, "\n".join(result["blockers"]))

    def test_complete_synthetic_protections(self):
        self.assertTrue(policy.audit(self.api, self.config)["ready"])

    def test_missing_app(self):
        self.config["release_app_id"] = None
        self.blocked("dedicated release GitHub App")

    def test_missing_reviewers(self):
        self.api.environments["release-stable"]["protection_rules"] = []
        self.blocked("release-stable: required reviewers missing")

    def test_admin_bypass_blocks(self):
        self.api.environments["release-staging"]["can_admins_bypass"] = True
        self.blocked("administrator approval bypass")

    def test_main_tag_is_not_main_branch(self):
        self.api.branches = [{"name": "main", "type": "tag"}]
        self.blocked("only the main branch")

    def test_disabled_rules_dont_count(self):
        self.api.rules[0]["enforcement"] = "disabled"
        self.blocked("creation must be restricted")

    def test_broad_bypass_blocks(self):
        self.api.rules[0]["bypass_actors"].append({"actor_id": 1, "actor_type": "RepositoryRole", "bypass_mode": "always"})
        self.blocked("creation must be restricted")

    def test_version_tag_update_bypass_blocks(self):
        self.api.rules[1]["bypass_actors"] = self.api.rules[0]["bypass_actors"]
        self.blocked("updates and deletions")

    def test_non_custodian_release_metadata_writer_blocks(self):
        self.api.people.append({"login": "writer", "permissions": {"push": True}})
        self.blocked("metadata remains writable by non-custodian writer")

    def test_cutover_audit_is_required(self):
        self.config["cutover_evidence_url"] = None
        self.blocked("Record the reviewed hosted cutover audit")

    def test_preflight_never_publishes(self):
        candidate, flow_policy, evidence = fixtures.fixture()
        self.api.candidate = lambda run_id, policy: (candidate, evidence)
        self.api.reviews = lambda candidate: evidence["reviews"]
        with patch.object(promote, "evaluate", return_value={"ready": True, "blockers": []}):
            result = promote.preflight(self.api, 123, flow_policy, self.config)
        self.assertTrue(result["ready"])
        self.assertIs(result["publication_enabled"], False)


if __name__ == "__main__":
    unittest.main()
