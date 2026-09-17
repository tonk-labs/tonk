#!/usr/bin/env python3
"""Identical-artifact calibration of S1; never an optimization verdict."""
import argparse
import datetime
import math
import os
from pathlib import Path
import random
import signal
import subprocess
import time

import ui_perf as perf
import perf_environment

METRIC = "navigation_to_usable_observed_ms"
BUILD_PROCESSES = {"cargo", "rustc", "clang", "clang++", "cc1", "wasm-opt", "ninja", "make"}


def sample_environment(run=perf_environment.command):
    """Bounded admission checks, not proof of a continuously idle host.

    Store only recognized build process names and PIDs, never command arguments
    or the inventory of personal applications.
    """
    processes = run(["ps", "-axo", "pid=,comm="])
    builds = []
    if processes is not None:
        for line in processes.splitlines():
            fields = line.strip().split(None, 1)
            if len(fields) == 2 and fields[0].isdecimal():
                name = Path(fields[1]).name
                if name in BUILD_PROCESSES:
                    builds.append({"pid": int(fields[0]), "name": name})
    return {"captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "process_check_available": processes is not None and bool(processes.strip()),
            "build_processes": builds,
            "power": perf_environment.power_settings(run(["pmset", "-g", "batt"]),
                                                     run(["pmset", "-g", "custom"]))}


def environment_problem(receipt, expected_power):
    if not receipt["process_check_available"]:
        return "process admission check unavailable"
    if receipt["build_processes"]:
        return "concurrent build processes detected"
    if any(value is None for value in receipt["power"].values()):
        return "power admission check unavailable"
    if receipt["power"] != expected_power:
        return "power configuration changed during collection"
    return None


def schedule(pairs, seed):
    perf.require(pairs >= 6 and pairs % 2 == 0, "use an even number of pairs >= 6")
    orders = ["AB", "BA"] * (pairs // 2)
    random.Random(seed).shuffle(orders)
    return [(pair, arm, order) for pair, order in enumerate(orders) for arm in order]


def control_delays(difference_ms, baseline_ms=None):
    """Match the hide/reobserve operation in both sensitivity-control arms.

    A 1 ms sham activates the same native control path as B. Ordinary A/A
    samples use no injection. Explicit baseline zero is diagnostic-only when
    the requested difference is positive.
    """
    if baseline_ms is None:
        baseline_ms = 1 if difference_ms else 0
    perf.require(type(difference_ms) is int and type(baseline_ms) is int
                 and difference_ms >= 0 and baseline_ms >= 0
                 and baseline_ms + difference_ms <= 2000, "invalid control delays")
    return {"A": baseline_ms, "B": baseline_ms + difference_ms}


def median_interval(values):
    """Exact sign-test interval for a population median, assuming independent pairs.

    Ties make this conservative. No Gaussian assumption or resampling of events
    within a session. Coverage is discrete and may exceed 95%.
    """
    n = len(values)
    eligible = [k for k in range(1, n // 2 + 1)
                if 2 * sum(math.comb(n, j) for j in range(k)) / 2**n <= .05]
    if not eligible:
        return None
    k = max(eligible)
    ordered = sorted(values)
    coverage = 1 - 2 * sum(math.comb(n, j) for j in range(k)) / 2**n
    return {"lower_ms": ordered[k - 1], "upper_ms": ordered[n - k],
            "coverage": coverage, "order_statistic": k}


def summarize(rows, pairs, budget_ms=100, expected_ms=0, control_baseline_ms=None):
    delays = control_delays(expected_ms, control_baseline_ms)
    matched = (expected_ms == 0 and delays["A"] == 0) or (expected_ms > 0 and delays["A"] > 0)
    good = [row for row in rows if row.get("status") == "ok"]
    differences = []
    orders = {"AB": [], "BA": []}
    for pair in range(pairs):
        members = [row for row in good if row["pair"] == pair]
        if len(members) != 2 or {r["arm"] for r in members} != {"A", "B"}:
            continue
        by_arm = {r["arm"]: r for r in members}
        difference = by_arm["B"]["metrics"][METRIC] - by_arm["A"]["metrics"][METRIC]
        differences.append(difference)
        orders[members[0]["order"]].append(difference)
    interval = median_interval(differences)
    complete = len(rows) == pairs * 2 and len(good) == pairs * 2 and len(differences) == pairs
    browsers = {perf.digest(r.get("browser")) for r in good}
    precision = bool(matched and complete and len(browsers) == 1 and interval
                     and interval["lower_ms"] >= expected_ms - budget_ms
                     and interval["upper_ms"] <= expected_ms + budget_ms)
    return {
        "kind": "headline-delay-control" if expected_ms else "identical-artifact-calibration",
        "expected_difference_ms": expected_ms, "attempted_runs": len(rows),
        "control_delays_ms": delays, "matched_control": matched,
        "successful_runs": len(good), "expected_pairs": pairs,
        "complete": complete, "consistent_browser": len(browsers) == 1,
        "paired_differences_ms": differences,
        "median_difference_ms": perf.quantile(differences, .5) if differences else None,
        "median_difference_interval": interval,
        "order_medians_ms": {order: perf.quantile(values, .5) if values else None
                             for order, values in orders.items()},
        "precision_budget_ms": budget_ms, "precision_pass": precision,
        "failures": [{"sequence": r["sequence"], "status": r.get("status"),
                      "error": r.get("error", r.get("failure_category"))}
                     for r in rows if r.get("status") != "ok"],
        "scope": "Local observed readiness only; A/A requires independent repeat and sensitivity validation",
    }


def run(args):
    delays = control_delays(args.delay_ms, getattr(args, "control_base_ms", None))
    artifact = Path(args.artifact).resolve(strict=True)
    runner = Path(args.runner).resolve(strict=True)
    server = Path(os.environ["TONK_UI_TEST_SERVER"]).resolve(strict=True)
    server_hash = perf.hashlib.sha256(server.read_bytes()).hexdigest()
    runner_hash = perf.hashlib.sha256(runner.read_bytes()).hexdigest()
    output = Path(args.output).resolve()
    perf.require(not output.exists(), "output exists; calibration evidence is immutable")
    acquisition = schedule(args.pairs, args.seed)
    identity = perf.artifact_digest(artifact)
    version = perf.read(artifact / "version.json")
    command = [str(runner), "performance::tests::it_profiles_supplied_artifact",
               "--exact", "--ignored", "--nocapture"]
    protocol = {**perf.baseline_protocol(), "kind": "s1-aa-calibration-v5",
                "pairs": args.pairs, "seed": args.seed, "schedule": acquisition,
                "precision_budget_ms": 100, "observer_diagnostics_version": 1,
                "headline_delay_A_ms": delays["A"], "headline_delay_B_ms": delays["B"],
                "expected_difference_ms": args.delay_ms,
                "headline_delay_method": "webdriver-inner-frame-visibility-matched-v2",
                "cooldown_seconds": args.cooldown,
                "environment_admission": {"version": 1, "checks": "before-and-after-each-session",
                                          "blocked_process_names": sorted(BUILD_PROCESSES),
                                          "require_unchanged_power": True}}
    protocol_hash = perf.digest(protocol)
    output.mkdir(parents=True)
    perf.write(output / "protocol.json", protocol)
    perf.write(output / "manifest.json", {"artifact_sha256": identity,
               "artifact": str(artifact), "version": version, "runner": command,
               "runner_sha256": runner_hash,
               "server": str(server), "server_sha256": server_hash,
               "protocol_sha256": protocol_hash})
    initial_environment = perf.capture_environment()
    perf.write(output / "environment-before.json", initial_environment)
    rows = []
    with (output / "runs.jsonl").open("x") as stream:
        for sequence, (pair, arm, order) in enumerate(acquisition):
            request = {"schema_version": 1, "measurement": perf.BASELINE_MEASUREMENT,
                       "scenario": "S1", "profile": "desktop",
                       "profile_settings": perf.PROFILES["desktop"], "fixture": perf.BASELINE_FIXTURE,
                       "fixture_seed": f"calibration-{args.seed}-{pair}", "slot": pair,
                       "fixture_sha256": perf.digest(perf.BASELINE_FIXTURE),
                       "calibration_headline_delay_ms": delays[arm]}
            request_path = output / f"request-{sequence:03d}.json"
            result_path = output / f"result-{sequence:03d}.json"
            perf.write(request_path, request)
            perf.require(perf.artifact_digest(artifact) == identity, "artifact changed before sample")
            perf.require(perf.hashlib.sha256(runner.read_bytes()).hexdigest() == runner_hash,
                         "runner changed before sample")
            perf.require(perf.hashlib.sha256(server.read_bytes()).hexdigest() == server_hash,
                         "test server changed before sample")
            time.sleep(args.cooldown)
            started = datetime.datetime.now(datetime.timezone.utc).isoformat()
            def record(result):
                row = {**result, "sequence": sequence, "pair": pair, "arm": arm,
                       "order": order, "started_at": started, "artifact_sha256": identity,
                       "protocol_sha256": protocol_hash}
                rows.append(row)
                stream.write(perf.json.dumps(row, allow_nan=False, sort_keys=True) + "\n")
                stream.flush()
                perf.write(output / "summary.json", summarize(rows, args.pairs, expected_ms=args.delay_ms,
                                                             control_baseline_ms=delays["A"]))
                print(f"{sequence + 1}/{len(acquisition)} {arm} {result.get('status')} "
                      f"{result.get('metrics', {}).get(METRIC)}", flush=True)

            before = sample_environment()
            perf.write(output / f"environment-{sequence:03d}-before.json", before)
            problem = environment_problem(before, initial_environment["power"])
            if problem:
                record({"status": "environment_blocked", "error": problem})
                break
            env = dict(os.environ, TONK_PERF_TRACE="0", TONK_PERF_REQUEST=str(request_path),
                       TONK_PERF_RESULT=str(result_path), TONK_UI_RELEASE_ARTIFACT=str(artifact))
            with (output / f"runner-{sequence:03d}.log").open("x") as log:
                process = subprocess.Popen(command, env=env, stdout=log,
                                           stderr=subprocess.STDOUT, start_new_session=True)
                try:
                    returncode = process.wait(timeout=180)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        process.wait(timeout=10)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        process.wait()
                    returncode = -1
            try:
                result = perf.read(result_path)
                perf.require(isinstance(result, dict), "non-object runner output")
                if result.get("status") == "ok":
                    perf.require(returncode == 0, "runner exited unsuccessfully")
                    for key in ("measurement", "fixture_seed", "slot", "fixture_sha256",
                                "scenario", "profile", "profile_settings"):
                        perf.require(result.get(key) == request[key], f"mismatched {key}")
                    perf.require(result.get("calibration_headline_delay_ms", 0)
                                 == request["calibration_headline_delay_ms"], "wrong calibration delay")
                    perf.require(all(result.get(k) is True for k in
                                     ("identity_verified", "fixture_verified", "profile_verified")),
                                 "missing validation")
                    perf.require(result.get("build_id") == version["build"], "wrong build")
                    perf.require(result.get("artifact_version") == version, "wrong artifact version")
                    perf.require(result.get("page_load_strategy") == "none", "wrong page-load strategy")
                    perf.require(isinstance(result.get("browser"), dict)
                                 and isinstance(result["browser"].get("product"), str)
                                 and result["browser"]["product"].startswith("Chrome/"), "missing Chrome identity")
                    perf.require(perf.number(result.get("metrics", {}).get(METRIC)), "missing timing")
                    perf.require(result.get("observer_diagnostics", {}).get("version") == 1,
                                 "missing observer diagnostics")
                    perf.require(result.get("functional_check", {}).get("share_menu_open") is True,
                                 "share guard missing")
                else:
                    perf.require(result.get("status") in {"timeout", "functional_failure", "harness_error"},
                                 "unknown runner failure")
                perf.require(perf.artifact_digest(artifact) == identity, "artifact changed during sample")
            except (OSError, ValueError, TypeError) as error:
                result = {"status": "harness_error", "error": str(error), "runner_exit": returncode}
            after = sample_environment()
            perf.write(output / f"environment-{sequence:03d}-after.json", after)
            problem = environment_problem(after, initial_environment["power"])
            if problem:
                result = {**result, "runner_status": result.get("status"),
                          "status": "environment_invalid", "error": problem}
            record(result)
            if result.get("status") in {"harness_error", "environment_invalid"}:
                break
    perf.write(output / "environment-after.json", perf.capture_environment())
    return 0 if summarize(rows, args.pairs, expected_ms=args.delay_ms,
                         control_baseline_ms=delays["A"])["precision_pass"] else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifact", required=True)
    parser.add_argument("--runner", required=True, help="Prebuilt native tonk-ui test executable")
    parser.add_argument("--output", required=True)
    parser.add_argument("--pairs", type=int, default=60)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument("--cooldown", type=float, default=2)
    parser.add_argument("--delay-ms", type=int, default=0,
                        help="Sensitivity control: additional headline hide duration for B")
    parser.add_argument("--control-base-ms", type=int,
                        help="Shared hide duration; defaults to 1 ms for sensitivity, 0 for ordinary A/A")
    args = parser.parse_args()
    perf.require(math.isfinite(args.cooldown) and 0 <= args.cooldown <= 30, "invalid cooldown")
    perf.require(0 <= args.delay_ms <= 2000, "invalid headline delay")
    return run(args)


if __name__ == "__main__":
    raise SystemExit(main())
