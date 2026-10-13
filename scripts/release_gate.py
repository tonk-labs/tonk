#!/usr/bin/env python3
"""Pure, fail-closed release policy. No network access or publishing capability."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import re
from urllib.parse import urlsplit


class InvalidEvidence(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise InvalidEvidence(message)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"),
                                     allow_nan=False).encode()).hexdigest()


def load(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            require(key not in result, f"duplicate JSON key: {key}")
            result[key] = value
        return result
    return json.loads(Path(path).read_text(), object_pairs_hook=unique)


def timestamp(value):
    parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    require(parsed.tzinfo is not None, "timestamp must include a timezone")
    return parsed


def safe_url(value):
    if not isinstance(value, str) or any(ord(c) < 32 for c in value):
        return False
    url = urlsplit(value)
    return (url.scheme == "https" and bool(url.hostname) and
            url.username is None and url.password is None)


def validate_context(context, policy):
    require(context.get("schema") == 1, "unsupported candidate schema")
    require(context.get("repository") == policy["repository"], "wrong repository")
    for key in ("source_sha", "controller_sha"):
        require(re.fullmatch(r"[0-9a-f]{40}", context.get(key, "")), f"invalid {key}")
    for key in ("run_id", "run_attempt"):
        require(type(context.get(key)) is int and context[key] > 0, f"invalid {key}")
    require(context.get("policy_digest") == digest(policy), "wrong policy digest")
    require(re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(-rc\.[1-9]\d*)?",
                         context.get("version", "")), "invalid release version")
    require(context.get("channel") in ("staging", "stable"), "invalid channel")
    require(("-rc." in context["version"]) == (context["channel"] == "staging"),
            "version does not match channel")


def indexed(records, label):
    result = {}
    for record in records:
        name = record["id"]
        require(name not in result, f"duplicate {label}: {name}")
        result[name] = record
    return result


def validate_policy(policy):
    require(policy.get("schema") == 1, "unsupported policy schema")
    flows = indexed(policy["flows"], "flow")
    require(bool(flows), "empty release policy")
    require(len(set(policy["checks"])) == len(policy["checks"]), "duplicate policy check")
    require(bool(policy["checks"]), "no required checks")
    require(policy["e2e_shards"] > 0, "no required shards")
    for flow in flows.values():
        require(flow["manual_group"] in policy["manual_groups"], "unknown manual group")
        require(bool(flow["tests"] or flow["checks"]), "flow has no automated evidence")
        require(set(flow["checks"]) <= set(policy["checks"]), "unknown flow check")
        require(len(set(flow["tests"])) == len(flow["tests"]), "duplicate mapped test")
    return flows


def summary_status(statuses):
    allowed = {"passed", "failed", "flaky", "blocked", "missing", "pending"}
    statuses = [status if status in allowed else "blocked" for status in statuses]
    for status in ("failed", "flaky", "blocked", "missing", "pending"):
        if status in statuses:
            return status
    return "passed"


def evaluate(candidate, policy, evidence, *, now=None):
    """Expected candidate comes from the controller, never from uploaded evidence.

    Only authenticated collectors may supply `reviews` and `staging` records.
    This evaluator checks content/identity; release_evidence verifies provenance.
    """
    now = now or datetime.now(timezone.utc)
    blockers, rows = [], []
    try:
        flows = validate_policy(policy)
        rows = [{"id": flow["id"], "title": flow["title"], "automated": "blocked", "manual": "missing",
                 "journeys": flow["journeys"], "tests": flow["tests"]} for flow in flows.values()]
        context = candidate["context"]
        validate_context(context, policy)
        require(evidence.get("context") == context, "evidence candidate identity mismatch")
        artifacts = candidate["artifacts"]
        require(set(artifacts) == set(policy["artifacts"]), "missing or unknown artifact")
        for name, value in artifacts.items():
            require(re.fullmatch(r"[0-9a-f]{64}", value), f"invalid artifact digest: {name}")
        candidate_digest = digest(candidate)
        require(evidence.get("candidate_digest") == candidate_digest, "artifact identity mismatch")
        checks = indexed(evidence.get("checks", []), "check")
        require(set(checks) <= set(policy["checks"]), "unknown check")
        for name in policy["checks"]:
            if checks.get(name, {}).get("status") != "passed":
                blockers.append(f"{name}: {checks.get(name, {}).get('status', 'missing')}")

        shards = indexed(evidence.get("shards", []), "shard")
        expected_shards = {str(n) for n in range(1, policy["e2e_shards"] + 1)}
        require(set(shards) <= expected_shards, "unknown shard")
        tests = {}
        for number in sorted(expected_shards):
            if number not in shards:
                blockers.append(f"E2E shard {number}: missing")
                continue
            shard = shards[number]
            require(shard["context"] == context, "shard candidate identity mismatch")
            require(shard["artifacts"] == {k: artifacts[k] for k in ("linux-cli", "web")},
                    "shard tested different artifacts")
            inventory = shard["inventory"]
            require(bool(inventory), f"empty inventory in shard {number}")
            require(len(inventory) == len(set(inventory)), "duplicate inventory test")
            results = indexed(shard["tests"], "test")
            require(set(results) <= set(inventory), "test not in discovered inventory")
            require(not (set(inventory) & tests.keys()), "test assigned to multiple shards")
            for name in inventory:
                result = results.get(name, {"id": name, "status": "missing"})
                tests[name] = result
                if result.get("status") != "passed" or result.get("attempts") != 1:
                    blockers.append(f"{name}: {result.get('status', 'missing')}, "
                                    f"attempts={result.get('attempts', 0)}")

        reviews = indexed(evidence.get("reviews", []), "review group")
        require(set(reviews) <= set(policy["manual_groups"]), "unknown review group")
        review_status = {}
        for group in policy["manual_groups"]:
            record = reviews.get(group)
            status = "missing"
            if record:
                age = (now - timestamp(record["verified_at"])).total_seconds()
                valid = (record.get("candidate_digest") == candidate_digest and
                         record.get("status") == "passed" and
                         isinstance(record.get("actor"), str) and bool(record["actor"]) and
                         safe_url(record.get("evidence_url")) and
                         0 <= age <= policy["manual_max_age_hours"] * 3600)
                if context["channel"] == "stable":
                    valid = valid and record.get("staging_digest") == digest(evidence.get("staging"))
                status = "passed" if valid else "blocked"
            review_status[group] = status
            if status != "passed":
                blockers.append(f"Manual verification {group}: {status}")

        if context["channel"] == "stable":
            staging = evidence.get("staging", {})
            require(staging.get("source_sha") == context["source_sha"], "staging source mismatch")
            require(staging.get("version", "").split("-rc.")[0] == context["version"],
                    "staging version mismatch")
            require(staging.get("candidate_digest") == context.get("staging_candidate_digest"),
                    "staging candidate mismatch")
            require(staging.get("current") is True, "staging candidate was superseded")
            require(set(staging.get("deliveries", {})) == {"web", "github", "npm"},
                    "incomplete staging publication")
            require(all(v == "passed" for v in staging["deliveries"].values()),
                    "failed staging publication")
            require(bool(staging.get("deployment_id")), "missing staging deployment identity")

        rows = []
        for flow in flows.values():
            missing = [name for name in flow["tests"] if tests.get(name, {}).get("status") != "passed"
                       or tests.get(name, {}).get("attempts") != 1]
            if missing:
                blockers.append(f"{flow['id']}: required mapped test missing or not passed: " + ", ".join(missing))
            statuses = [tests.get(name, {}).get("status", "missing") for name in flow["tests"]]
            if any(tests.get(name, {}).get("attempts", 1) > 1 for name in flow["tests"]):
                statuses.append("flaky")
            if any(tests.get(name, {}).get("status") == "passed" and
                   tests.get(name, {}).get("attempts") != 1 for name in flow["tests"]):
                statuses.append("blocked")
            statuses += [checks.get(name, {}).get("status", "missing") for name in flow["checks"]]
            rows.append({"id": flow["id"], "title": flow["title"],
                         "automated": summary_status(statuses),
                         "manual": review_status[flow["manual_group"]],
                         "journeys": flow["journeys"], "tests": flow["tests"]})
    except (InvalidEvidence, KeyError, TypeError, ValueError, AttributeError) as error:
        blockers.append(f"Invalid evidence: {error}")
    return {"schema": 1, "ready": not blockers, "blockers": blockers, "flows": rows,
            "candidate": candidate, "evaluated_at": now.isoformat()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--policy", default="release/flows.json")
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    result = evaluate(load(args.candidate), load(args.policy), load(args.evidence))
    Path(args.output).write_text(json.dumps(result, indent=2) + "\n")
    print("Ready" if result["ready"] else "Blocked: " + "; ".join(result["blockers"]))
    return 0 if result["ready"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
