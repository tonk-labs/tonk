#!/usr/bin/env python3
"""Compare native 16/256 codegen units with warm dependencies and fresh sources.

Run inside `nix develop .#ci`. Outputs stay outside the checkout; normal profiles
are untouched. Start with --scope worker; use --scope workspace for CI coverage.
The worker probe has a smaller feature graph and is not a whole-CI speed estimate.
"""

import argparse
import datetime
import json
import pathlib
import platform
import shutil
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scope", choices=["worker", "workspace"], default="worker")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--rounds", type=int, default=1)
    parser.add_argument("--cores", type=int, default=4)
    parser.add_argument("--units", type=int, choices=[16, 256], nargs="+", default=[16, 256])
    parser.add_argument("--opt-level", type=int, choices=[0, 1], default=1)
    options = parser.parse_args()
    if options.rounds < 1 or options.cores < 1:
        parser.error("rounds and cores must be positive")
    repo = pathlib.Path(__file__).resolve().parents[1]
    output = options.output.resolve()
    if output == repo or repo in output.parents:
        parser.error("output must be outside the repository")
    output.mkdir(parents=True, exist_ok=False)
    nonce = datetime.datetime.now(datetime.timezone.utc).isoformat()
    results = {
        "scope": options.scope,
        "host": platform.platform(),
        "cores": options.cores,
        "opt_level": options.opt_level,
        "nonce": nonce,
        "revision": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip(),
        "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=repo, text=True)),
        "dependencies": {},
        "trials": [],
    }

    def expression(units, stage, trial):
        return f'''import {json.dumps(str(repo / "nix/native-codegen-benchmark.nix"))} {{
          repo = {json.dumps(str(repo))}; units = {units}; optLevel = {options.opt_level};
          nonce = {json.dumps(nonce + ":" + str(trial))};
          scope = {json.dumps(options.scope)}; stage = {json.dumps(stage)};
        }}'''

    def build(units, stage, trial):
        label = f"{stage}-{units}-{trial}"
        print(f"Building {label}", flush=True)
        start = time.monotonic()
        with (output / f"{label}.log").open("w") as log:
            run = subprocess.run([
                "nix", "build", "--impure", "--no-link", "--print-out-paths", "-L",
                "--max-jobs", "1", "--cores", str(options.cores),
                "--expr", expression(units, stage, trial),
            ], cwd=repo, text=True, stdout=subprocess.PIPE, stderr=log, check=True)
        return pathlib.Path(run.stdout.strip()), time.monotonic() - start

    # Warm BOTH dependency graphs before measuring either source rebuild.
    for units in dict.fromkeys(options.units):
        path, seconds = build(units, "dependencies", 0)
        results["dependencies"][str(units)] = {"path": str(path), "warmup_seconds": seconds}
        (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    for trial in range(options.rounds):
        # Reverse the order on alternate rounds to expose ordering effects.
        for units in (options.units if trial % 2 == 0 else reversed(options.units)):
            archive, seconds = build(units, "archive", trial)
            label = f"archive-{units}-{trial}"
            shutil.copytree(archive / "cargo-timings", output / label)
            start = time.monotonic()
            with (output / f"{label}-tests.log").open("w") as log:
                run = subprocess.run([
                    "cargo", "nextest", "run", "--archive-file",
                    str(archive / "tests-native-ci.tar.zst"), "--workspace-remap", str(repo),
                    "--status-level", "fail", "--final-status-level", "fail",
                ], cwd=repo, stdout=log, stderr=subprocess.STDOUT)
            results["trials"].append({
                "units": units, "round": trial, "build_seconds": seconds,
                "test_seconds": time.monotonic() - start, "test_exit_code": run.returncode,
                "archive": str(archive),
                "archive_bytes": (archive / "tests-native-ci.tar.zst").stat().st_size,
            })
            (output / "results.json").write_text(json.dumps(results, indent=2) + "\n")
            print(json.dumps(results["trials"][-1]), flush=True)
            if run.returncode:
                raise SystemExit("Tests failed; inspect the saved log before changing profiles.")


if __name__ == "__main__":
    main()
