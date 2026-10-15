#!/usr/bin/env python3
"""Collect structured release evidence; never infer a pass from absent output."""
from __future__ import annotations

import argparse
from datetime import datetime, timezone
import io
import json
import os
from pathlib import Path
import re
import subprocess
import xml.etree.ElementTree as ET
import zipfile

from release_artifacts import file_digest, tree_digest
from release_gate import digest, evaluate, indexed, load, require, safe_url, validate_context, validate_policy


def save(path, value):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2) + "\n")


def inventory(data):
    names = []
    for suite in data["rust-suites"].values():
        require(suite.get("status") == "listed", "test binary was not listed")
        for name, case in suite["testcases"].items():
            if case["filter-match"]["status"] == "matches":
                require(not case["ignored"], "ignored test selected for release")
                names.append(suite["binary-id"] + "::" + name)
    require(bool(names), "empty discovered test inventory")
    require(len(names) == len(set(names)), "duplicate discovered test")
    return sorted(names)


def junit(data):
    # Only identities/statuses survive parsing: raw test output may contain
    # account data or callback URLs and must not be copied into public reports.
    require(b"<!DOCTYPE" not in data.upper() and b"<!ENTITY" not in data.upper(), "XML entities forbidden")
    root = ET.fromstring(data)
    require(root.tag == "testsuites", "expected nextest testsuites report")
    records = []
    for suite in root.findall("testsuite"):
        cases = suite.findall("testcase")
        require(int(suite.attrib["tests"]) == len(cases), "incomplete JUnit suite")
        for case in cases:
            require(case.attrib["classname"] == suite.attrib["name"], "JUnit binary identity mismatch")
            tags = [node.tag for node in case]
            retries = sum(tag in ("flakyFailure", "flakyError", "rerunFailure", "rerunError") for tag in tags)
            status = "passed"
            if "skipped" in tags:
                status = "blocked"
            elif "failure" in tags or "error" in tags:
                status = "failed"
            elif retries:
                status = "flaky"
            require(set(tags) <= {"failure", "error", "skipped", "flakyFailure", "flakyError",
                                  "rerunFailure", "rerunError", "system-out", "system-err", "properties"},
                    "unknown JUnit result element")
            records.append({"id": suite.attrib["name"] + "::" + case.attrib["name"],
                            "status": status, "attempts": retries + 1})
    require(bool(records), "empty JUnit report")
    indexed(records, "JUnit test")
    return records


def collect_shard(context, number, inventory_path, junit_path, cli, web):
    return {"id": str(number), "context": context,
            "artifacts": {"linux-cli": file_digest(cli), "web": tree_digest(web)},
            "inventory": inventory(load(inventory_path)),
            "tests": junit(Path(junit_path).read_bytes())}


class GitHub:
    """Read-only GitHub transport. Writes belong to the promotion controller."""
    def __init__(self, repository):
        require(re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository), "invalid repository")
        self.repository = repository

    def raw(self, path):
        return subprocess.run(["gh", "api", f"repos/{self.repository}/{path}"],
                              check=True, stdout=subprocess.PIPE).stdout

    def get(self, path):
        return json.loads(self.raw(path))

    def pages(self, path, key):
        result = []
        separator = "&" if "?" in path else "?"
        for page in range(1, 101):
            values = self.get(f"{path}{separator}per_page=100&page={page}")
            values = values[key] if key else values
            result.extend(values)
            if len(values) < 100:
                return result
        raise ValueError("GitHub pagination limit exceeded; refusing partial evidence")

    def run(self, run_id, workflow):
        require(str(run_id).isdigit(), "invalid run ID")
        run = self.get(f"actions/runs/{run_id}")
        require(run["repository"]["full_name"] == self.repository, "wrong run repository")
        require(run["path"] == f".github/workflows/{workflow}", "wrong evidence workflow")
        require(run["event"] == "workflow_dispatch" and run["head_branch"] == "main", "untrusted evidence ref/event")
        require(run["status"] == "completed", "evidence run is not complete")
        return run

    def artifact(self, run, name):
        matches = [item for item in self.pages(f"actions/runs/{run['id']}/artifacts", "artifacts")
                   if item["name"] == name]
        require(len(matches) == 1 and not matches[0]["expired"], "missing, duplicate or expired evidence artifact")
        item = matches[0]
        require(item["workflow_run"]["id"] == run["id"] and
                item["workflow_run"]["head_sha"] == run["head_sha"], "artifact provenance mismatch")
        require(item["size_in_bytes"] <= 20_000_000, "evidence artifact too large")
        raw = self.raw(f"actions/artifacts/{item['id']}/zip")
        # GitHub's digest is the uploaded artifact archive, not a JSON digest.
        import hashlib
        require(item.get("digest") == "sha256:" + hashlib.sha256(raw).hexdigest(), "artifact download digest mismatch")
        with zipfile.ZipFile(io.BytesIO(raw)) as archive:
            require(sum(member.file_size for member in archive.infolist()) <= 20_000_000,
                    "expanded evidence artifact too large")
            names = archive.namelist()
            require(len(names) == len(set(names)), "duplicate artifact file")
            return {name: archive.read(name) for name in names if name.endswith(".json")}

    def candidate(self, run_id, policy):
        run = self.run(run_id, "release-readiness.yml")
        files = self.artifact(run, f"release-candidate-{run['run_attempt']}")
        candidate = json.loads(files["candidate.json"])
        evidence = json.loads(files["evidence.json"])
        context = candidate["context"]
        validate_context(context, policy)
        require(context["controller_sha"] == run["head_sha"] and
                context["run_id"] == run["id"] and context["run_attempt"] == run["run_attempt"],
                "candidate run identity mismatch")
        # A failed report job is expected while human verification is missing.
        # Automated checks must be authenticated independently of that failure.
        require(run["conclusion"] in ("success", "failure"), "cancelled candidate run")
        return candidate, evidence

    def authorized_reviewer(self, actor):
        require(re.fullmatch(r"[A-Za-z0-9_-]+", actor), "invalid reviewer")
        access = self.get(f"collaborators/{actor}/permission")
        # The legacy permission field maps maintain to write.
        permission = access.get("role_name", access.get("permission"))
        require(permission in ("admin", "maintain"), "reviewer requires maintain or admin permission")

    def reviews(self, candidate):
        records = {}
        runs = self.pages("actions/workflows/release-verify.yml/runs?event=workflow_dispatch&branch=main", "workflow_runs")
        for value in sorted(runs, key=lambda item: item["id"], reverse=True):
            if value["status"] != "completed" or value["conclusion"] != "success":
                continue
            run = self.run(value["id"], "release-verify.yml")
            files = self.artifact(run, "release-verification")
            record = json.loads(files["verification.json"])
            if record["candidate_digest"] != digest(candidate):
                continue
            require(run["run_attempt"] == 1, "verification reruns cannot replace evidence; record a new verification")
            require(record["actor"] == run["actor"]["login"], "verification actor mismatch")
            self.authorized_reviewer(record["actor"])
            records.setdefault(record["id"], record)
        return list(records.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    init = sub.add_parser("context")
    init.add_argument("--source", required=True)
    init.add_argument("--version", required=True)
    init.add_argument("--policy", default="release/flows.json")
    init.add_argument("--output", required=True)
    shard = sub.add_parser("shard")
    for name in ("context", "number", "inventory", "junit", "cli", "web", "output"):
        shard.add_argument("--" + name, required=True)
    identity = sub.add_parser("identity")
    for name in ("context", "name", "path", "output"):
        identity.add_argument("--" + name, required=True)
    identity.add_argument("--tree", action="store_true")
    assemble = sub.add_parser("assemble")
    for name in ("context", "policy", "artifacts", "checks", "output"):
        assemble.add_argument("--" + name, required=True)
    verify = sub.add_parser("verify")
    for name in ("candidate-run", "group", "result", "evidence-url", "notes", "output"):
        verify.add_argument("--" + name, required=True)
    verify.add_argument("--policy", default="release/flows.json")
    args = parser.parse_args()
    if args.command == "context":
        require(os.environ["GITHUB_REF"] == "refs/heads/main", "run the controller from main")
        policy = load(args.policy)
        validate_policy(policy)
        context = {"schema": 1, "repository": os.environ["GITHUB_REPOSITORY"],
                   "source_sha": args.source, "controller_sha": os.environ["GITHUB_SHA"],
                   "run_id": int(os.environ["GITHUB_RUN_ID"]), "run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"]),
                   "version": args.version, "channel": "staging" if "-rc." in args.version else "stable",
                   "policy_digest": digest(policy)}
        validate_context(context, policy)
        subprocess.run(["git", "merge-base", "--is-ancestor", args.source, "origin/main"], check=True)
        # Stable preparation must be derived from an authenticated staging receipt.
        # The rollout path is intentionally unavailable until that collector exists.
        require(context["channel"] == "staging", "stable candidate preparation requires the staging receipt collector")
        save(args.output, context)
    elif args.command == "shard":
        save(args.output, collect_shard(load(args.context), args.number, args.inventory, args.junit, args.cli, args.web))
    elif args.command == "identity":
        save(args.output, {"context": load(args.context), "name": args.name,
                           "sha256": tree_digest(args.path) if args.tree else file_digest(args.path)})
    elif args.command == "assemble":
        context = load(args.context)
        root = Path(args.artifacts)
        artifacts = {}
        for name in ("linux-cli", "macos-cli", "web"):
            path = root / f"release-evidence-{name}-{context['run_attempt']}" / "identity.json"
            if path.exists():
                record = load(path)
                require(record["context"] == context and record["name"] == name, "build identity mismatch")
                artifacts[name] = record["sha256"]
        candidate = {"context": context, "artifacts": artifacts}
        needs = load(args.checks)
        evidence = {"context": context, "candidate_digest": digest(candidate), "reviews": [],
                    "checks": [{"id": key, "status": {"success": "passed", "failure": "failed",
                                                       "skipped": "missing", "cancelled": "blocked"}.get(value["result"], "blocked")}
                               for key, value in needs.items() if key in load(args.policy)["checks"]],
                    "shards": [load(path) for path in sorted(root.glob(f"release-evidence-e2e-*-{context['run_attempt']}/shard.json"))]}
        save(Path(args.output) / "candidate.json", candidate)
        save(Path(args.output) / "evidence.json", evidence)
    elif args.command == "verify":
        require(os.environ["GITHUB_REF"] == "refs/heads/main" and
                os.environ["GITHUB_EVENT_NAME"] == "workflow_dispatch", "verification must run from main")
        require(os.environ["GITHUB_RUN_ATTEMPT"] == "1", "record a new verification instead of re-running")
        policy = load(args.policy)
        require(args.group in policy["manual_groups"], "unknown manual group")
        require(args.result in ("passed", "failed", "blocked"), "invalid manual result")
        require(safe_url(args.evidence_url), "evidence must have a safe HTTPS URL")
        require(0 < len(args.notes.strip()) <= 2000, "provide verification notes (up to 2000 characters)")
        api = GitHub(policy["repository"])
        actor = os.environ["GITHUB_ACTOR"]
        api.authorized_reviewer(actor)
        candidate, evidence = api.candidate(args.candidate_run, policy)
        record = {"id": args.group, "candidate_digest": digest(candidate), "status": args.result,
                  "actor": actor, "verified_at": datetime.now(timezone.utc).isoformat(),
                  "evidence_url": args.evidence_url, "notes": args.notes,
                  "run_id": int(os.environ["GITHUB_RUN_ID"])}
        if candidate["context"]["channel"] == "stable":
            require("staging" in evidence, "missing staging receipt")
            record["staging_digest"] = digest(evidence["staging"])
        evidence["reviews"] = [value for value in api.reviews(candidate) if value["id"] != args.group] + [record]
        save(Path(args.output) / "verification.json", record)
        from release_report import write
        write(evaluate(candidate, policy, evidence), args.output)


if __name__ == "__main__":
    main()
