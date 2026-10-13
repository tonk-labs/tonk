#!/usr/bin/env python3
"""Fail closed until Test succeeds for this exact pushed release tag and SHA."""

import json
import os
import sys
import time
import urllib.parse
import urllib.request

WORKFLOW = ".github/workflows/test.yml"


def matches(run, sha, tag):
    return (
        run.get("head_sha") == sha
        and run.get("head_branch") == tag
        and run.get("event") == "push"
        and run.get("path") == WORKFLOW
    )


def latest(runs, sha, tag):
    candidates = [run for run in runs if matches(run, sha, tag)]
    return max(candidates, key=lambda run: (run["id"], run["run_attempt"]), default=None)


def list_runs(get, sha, tag):
    # GitHub returns latest attempts by default. Paginate so a busy repository
    # cannot hide the matching tag behind unrelated runs at the same revision.
    runs = []
    for page in range(1, 11):
        query = urllib.parse.urlencode(
            {"head_sha": sha, "event": "push", "branch": tag, "per_page": 100, "page": page}
        )
        batch = get(f"actions/workflows/test.yml/runs?{query}")["workflow_runs"]
        runs.extend(batch)
        if len(batch) < 100:
            return runs
    raise RuntimeError("verification run pagination limit exceeded")


def wait(get, sha, ref, timeout=10800, interval=30, clock=time.monotonic, sleep=time.sleep):
    if not ref.startswith("refs/tags/v"):
        raise ValueError("verification requires a pushed v* tag ref")
    tag = ref.removeprefix("refs/tags/")
    deadline = clock() + timeout
    while clock() < deadline:
        run = latest(list_runs(get, sha, tag), sha, tag)
        if run:
            # Refresh the latest attempt, never accept a cached older success
            # over a newer queued/running/failed rerun of this workflow run.
            current = get(f"actions/runs/{run['id']}")
            if not matches(current, sha, tag) or current["id"] != run["id"]:
                raise RuntimeError("verification run identity changed")
            if current["run_attempt"] < run["run_attempt"]:
                raise RuntimeError("verification API returned an older attempt")
            if current["status"] == "completed":
                if current["conclusion"] != "success":
                    raise RuntimeError(f"tag verification ended with {current['conclusion']}")
                print(f"Verified {ref} at {sha}: run {current['id']} attempt {current['run_attempt']}")
                return current
        remaining = deadline - clock()
        if remaining > 0:
            sleep(min(interval, remaining))
    raise TimeoutError(f"no successful tag verification within {timeout}s")


def main():
    if os.environ["GITHUB_EVENT_NAME"] != "push":
        raise ValueError("release verification only accepts push events")
    base = os.environ.get("GITHUB_API_URL", "https://api.github.com")
    repository = os.environ["GITHUB_REPOSITORY"]

    def get(path):
        request = urllib.request.Request(
            f"{base}/repos/{repository}/{path}",
            headers={
                "Accept": "application/vnd.github+json",
                "Authorization": f"Bearer {os.environ['GH_TOKEN']}",
                "X-GitHub-Api-Version": "2022-11-28",
            },
        )
        # API/network failures fail closed; retry the publisher workflow after
        # recovery. Neither response bodies nor credentials are printed.
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)

    wait(get, os.environ["GITHUB_SHA"], os.environ["GITHUB_REF"])


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"::error::Tag verification refused publication: {error}", file=sys.stderr)
        sys.exit(1)
