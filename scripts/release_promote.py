#!/usr/bin/env python3
"""Read-only promotion preflight. Delivery remains disabled until cutover proof."""
import argparse
from pathlib import Path
import subprocess

from release_evidence import GitHub, save
from release_gate import evaluate, load
from release_policy import audit
from release_report import write


def preflight(api, run_id, policy, configuration):
    candidate, evidence = api.candidate(run_id, policy)
    evidence["reviews"] = api.reviews(candidate)
    result = evaluate(candidate, policy, evidence)
    protections = audit(api, configuration)
    result["blockers"].extend(protections["blockers"])
    result["ready"] = not result["blockers"]
    result["publication_enabled"] = False
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate-run", required=True)
    parser.add_argument("--policy", default="release/flows.json")
    parser.add_argument("--protections", default="release/github/policy.json")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    policy = load(args.policy)
    try:
        result = preflight(GitHub(policy["repository"]), args.candidate_run, policy, load(args.protections))
    except (ValueError, KeyError, subprocess.CalledProcessError) as error:
        result = evaluate({"context": {}, "artifacts": {}}, policy, {})
        result["blockers"].insert(0, f"Could not authenticate complete candidate evidence: {error}")
        result["publication_enabled"] = False
    write(result, args.output)
    save(Path(args.output) / "preflight.json", result)
    raise SystemExit(0 if result["ready"] else 1)
