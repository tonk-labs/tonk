#!/usr/bin/env python3
"""Read-only audit of the external protections required for release promotion."""
import argparse
import json
import subprocess
from pathlib import Path

from release_evidence import GitHub
from release_gate import load


def audit(api, configuration):
    blockers = []
    app_id = configuration.get("release_app_id")
    if type(app_id) is not int or app_id <= 0:
        blockers.append("Configure the dedicated release GitHub App ID")
    for name in ("release-staging", "release-stable"):
        try:
            env = api.get(f"environments/{name}")
            rules = env.get("protection_rules", [])
            if not any(rule["type"] == "required_reviewers" and rule.get("reviewers") for rule in rules):
                blockers.append(f"{name}: required reviewers missing")
            if env.get("can_admins_bypass") is not False:
                blockers.append(f"{name}: administrator approval bypass must be disabled")
            if env.get("deployment_branch_policy") != {"protected_branches": False, "custom_branch_policies": True}:
                blockers.append(f"{name}: restrict deployment to the main branch")
            else:
                branches = api.pages(f"environments/{name}/deployment-branch-policies", "branch_policies")
                if len(branches) != 1 or branches[0].get("name") != "main" or branches[0].get("type") != "branch":
                    blockers.append(f"{name}: only the main branch may deploy")
        except (KeyError, ValueError, subprocess.CalledProcessError):
            blockers.append(f"{name}: environment protection could not be verified")

    rulesets = [api.get(f"rulesets/{item['id']}") for item in api.pages("rulesets", None)]
    active = [rule for rule in rulesets if rule.get("enforcement") == "active" and rule.get("target") == "tag"
              and rule.get("conditions", {}).get("ref_name") == {"include": ["refs/tags/v*"], "exclude": []}]
    app_bypass = [{"actor_id": app_id, "actor_type": "Integration", "bypass_mode": "always"}]
    if not any(rule.get("bypass_actors") == app_bypass and
               any(item["type"] == "creation" for item in rule["rules"]) for rule in active):
        blockers.append("v* creation must be restricted to the dedicated release app")
    if not any(not rule.get("bypass_actors") and
               {"update", "deletion"} <= {item["type"] for item in rule["rules"]} for rule in active):
        blockers.append("v* updates and deletions must be forbidden without a normal bypass")

    # Tag rules do not protect release assets/latest. Audit every collaborator
    # rather than accepting a claim that metadata has been isolated.
    custodians = set(configuration.get("release_custodians", []))
    if not custodians:
        blockers.append("Declare the release custodians who retain GitHub Release metadata access")
    for person in api.pages("collaborators?affiliation=all", None):
        permissions = person.get("permissions", {})
        if any(permissions.get(role) for role in ("push", "maintain", "admin")) and person["login"] not in custodians:
            blockers.append(f"Release metadata remains writable by non-custodian {person['login']}")
    # API-visible policy cannot prove credential rotation or npm/Cloudflare
    # scope. Keep an explicit cutover hold until the recorded hosted audit is
    # reviewed. This is never set from a workflow_dispatch checkbox.
    if not (configuration.get("cutover_evidence_url") or "").startswith("https://github.com/"):
        blockers.append("Record the reviewed hosted cutover audit before enabling delivery")
    return {"ready": not blockers, "blockers": blockers}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--config", default="release/github/policy.json")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    result = audit(GitHub(args.repo), load(args.config))
    Path(args.output).write_text(json.dumps(result, indent=2) + "\n")
    for reason in result["blockers"]:
        print(reason)
    raise SystemExit(0 if result["ready"] else 1)
