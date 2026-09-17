"""Frozen-protocol paired performance campaigns. Python standard library only."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import signal
import subprocess
import sys

from perf_environment import capture as capture_environment

SCHEMA = 1
DRAWS = 10000
SEED = 20260916
KINDS = {"startup", "paint", "completion", "latency", "offline", "resource"}
PROFILES = {
    "desktop": {"cpu_slowdown": 1, "latency_ms": 0, "download_bytes_per_second": -1, "upload_bytes_per_second": -1, "viewport_width": 1280, "viewport_height": 800},
    "constrained": {"cpu_slowdown": 4, "latency_ms": 150, "download_bytes_per_second": 200000, "upload_bytes_per_second": 93750, "viewport_width": 1280, "viewport_height": 800},
}
# Fixed minimum coverage; an experiment may add guards, never omit these to win.
REQUIRED_METRICS = {
    "S1": {"navigation_to_usable_ms": "startup", "first_input_ms": "paint", "requests": "resource", "bytes": "resource"},
    "S2": {"local_content_ready_ms": "startup"},
    "S3": {"route_ready_ms": "startup", "menu_ready_ms": "completion", "input_to_next_paint_ms": "paint"},
    "S4": {"edit_completed_ms": "completion", "input_to_next_paint_ms": "paint"},
    "S5": {"editor_usable_ms": "completion", "typing_latency_ms": "paint"},
    "S6": {"active_query_ms": "completion", "edit_completed_ms": "completion", "visible_freshness_ms": "latency", "idle_freshness_ms": "latency", "dirty_push_ms": "latency"},
    "S7": {"guest_ready_ms": "startup", "root_preparations": "resource", "payload_bytes": "resource"},
    "S8": {"offline_ready_ms": "offline"},
}
FOOTPRINT_METRICS = {"total_transferred_bytes", "total_compiled_bytes", "peak_memory_bytes", "settled_memory_bytes", "cpu_ms"}
RUNNER = ["nix", "develop", ".", "--command", "cargo", "test", "-p", "tonk-ui", "--features", "integration-tests", "performance::tests::it_profiles_supplied_artifact", "--", "--exact", "--ignored", "--nocapture"]
BASELINE_RUNS = 10
BASELINE_MEASUREMENT = "navigation-to-usable-observed-v3"
BASELINE_FIXTURE = {
    "recipe": "fresh-anonymous-nested-welcome-v1",
    "selector": ".wp-h1",
    "input_sequence": "open-share-menu-v1",
    "dataset_sha256": "ef4c970fce9dc639ad8770e8ee025de886e753fdd942f451400ec85bf6e5e55a",
    "cache_state": "fresh-profile-empty-http-and-service-worker",
}


class Invalid(ValueError):
    """Configuration, identity, or harness failure (exit 1)."""


def require(condition, message):
    if not condition:
        raise Invalid(message)


def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()


def read(path):
    with open(path) as stream:
        return json.load(stream, parse_constant=lambda value: (_ for _ in ()).throw(Invalid("non-finite JSON")))


def write(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n")


def number(value):
    return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value) and value >= 0


def validate(config):
    require(isinstance(config, dict), "protocol must be an object")
    require(config.get("schema_version") == SCHEMA, "unsupported protocol schema")
    require(type(config.get("frozen")) is bool, "frozen must be explicit")
    require(type(config.get("diagnostics_enabled")) is bool, "diagnostics_enabled must be an explicit boolean")
    require(config.get("bootstrap") == {"draws": DRAWS, "seed": SEED, "quantile": "linear", "pairing": "cycle/block/seed/slot"}, "statistical contract differs from plan")
    selections = config.get("selections")
    require(isinstance(selections, list) and selections, "explicit selections required")
    seen = set()
    for selection in selections:
        require(isinstance(selection, dict), "selection must be an object")
        key = (selection.get("scenario"), selection.get("profile"))
        require(key[0] in {f"S{i}" for i in range(1, 9)} and key[1] in PROFILES, "unknown scenario/profile")
        require(key not in seen, "duplicate scenario/profile")
        seen.add(key)
        require(selection.get("profile_settings") == PROFILES[key[1]], "profile settings must match declared profile")
        fixture = selection.get("fixture", {})
        require(isinstance(fixture, dict), "fixture must be an object")
        require(all(isinstance(fixture.get(field), str) and fixture[field] for field in ("recipe", "selector", "input_sequence", "dataset_sha256", "cache_state")), "fixture recipe, selector, input sequence, digest and cache state required")
        require(len(fixture["dataset_sha256"]) == 64 and all(c in "0123456789abcdef" for c in fixture["dataset_sha256"]), "invalid fixture digest")
        require(isinstance(selection.get("metrics"), dict) and selection["metrics"], "explicit metrics required")
        for metric, spec in selection["metrics"].items():
            require(isinstance(spec, dict), "metric specification must be an object")
            require(isinstance(metric, str) and metric and spec.get("kind") in KINDS, "unknown metric kind")
            if "zero_absolute_budget" in spec:
                require(number(spec["zero_absolute_budget"]), "invalid absolute zero budget")
            if "absolute_ceiling" in spec:
                require(number(spec["absolute_ceiling"]), "invalid absolute metric ceiling")
    experiments = config.get("experiments")
    require(isinstance(experiments, dict), "experiments must be an object")
    for name, experiment in experiments.items():
        require(isinstance(experiment, dict), "experiment must be an object")
        require(name and experiment.get("hypothesis") and experiment.get("classification_reason"), "experiment hypothesis/classification required")
        primary = experiment.get("primary", {})
        require(isinstance(primary, dict), "primary must be an object")
        spec = lookup(config, primary)
        require(set(primary) == {"scenario", "profile", "metric"}, "primary endpoint must contain exactly scenario/profile/metric")
        require(spec["kind"] in {"startup", "paint", "completion"}, "primary must use practical user latency gate")
        require(isinstance(experiment.get("affected"), list) and isinstance(experiment.get("sentinels"), list), "affected/sentinel classifications required")
        require(all(isinstance(x, dict) for x in experiment["affected"] + experiment["sentinels"]), "classification must be objects")
        classified = [(x.get("scenario"), x.get("profile")) for x in experiment["affected"] + experiment["sentinels"]]
        require(len(classified) == len(set(classified)) and set(classified) == seen, "classify every selection exactly once")
        require((primary["scenario"], primary["profile"]) in [(x["scenario"], x["profile"]) for x in experiment["affected"]], "primary must be affected")
    return config


def lookup(config, endpoint):
    for selection in config["selections"]:
        if all(selection[key] == endpoint.get(key) for key in ("scenario", "profile")):
            require(endpoint.get("metric") in selection["metrics"], "unknown primary metric")
            return selection["metrics"][endpoint["metric"]]
    raise Invalid("unknown endpoint selection")


def quantile(values, q):
    values = sorted(values)
    require(bool(values), "empty distribution")
    index = (len(values) - 1) * q
    lo = int(index)
    hi = min(lo + 1, len(values) - 1)
    return values[lo] + (values[hi] - values[lo]) * (index - lo)


def budget(kind, a, q, zero_absolute=None):
    if a == 0 and zero_absolute is not None:
        return zero_absolute
    if kind == "resource":
        return None if a == 0 else 0.05 * a
    if kind == "offline":
        return max(250, 0.05 * a)
    return max(8, 0.05 * a) if q == 0.5 else max(16, 0.1 * a)


def target(kind, a):
    absolute, relative = {"startup": (100, .05), "paint": (16, .1), "completion": (25, .1)}[kind]
    return max(absolute, relative * a)


def bootstrap(pairs, statistic, draws=DRAWS):
    """Resample cycles, block pairs, then matched slots; never event-level data."""
    flattened = [slot for cycle in pairs for pair in cycle for slot in pair]
    if len(set(flattened)) == 1:
        value = statistic([v[0] for v in flattened], [v[1] for v in flattened])
        return None if value is None else [value, value]
    rng = random.Random(SEED)
    distribution = []
    for _ in range(draws):
        a, b = [], []
        for _ in pairs:
            cycle = pairs[rng.randrange(len(pairs))]
            for _ in cycle:
                pair = cycle[rng.randrange(len(cycle))]
                for _ in pair:
                    left, right = pair[rng.randrange(len(pair))]
                    a.append(left)
                    b.append(right)
        value = statistic(a, b)
        if value is None:
            return None
        distribution.append(value)
    return [quantile(distribution, .025), quantile(distribution, .975)]


def schedule(phase, sentinel=False):
    cycles, slots = (3, 10) if phase in {"confirm", "calibrate"} and not sentinel else (1, 5)
    return [(cycle, block, arm, slot, f"{cycle}:{pair}:{slot}")
            for cycle in range(cycles)
            for block, arm, pair in [(0, "A", 0), (1, "B", 0), (2, "B", 1), (3, "A", 1)]
            for slot in range(slots)]


def selection_sentinel(config, experiment, selection):
    if experiment is None:
        return False
    return any(all(item[k] == selection[k] for k in ("scenario", "profile")) for item in config["experiments"][experiment]["sentinels"])


def analyze(config, manifest, runs):
    validate(config)
    require(isinstance(manifest, dict) and manifest.get("schema_version") == SCHEMA, "unsupported campaign manifest schema")
    require(isinstance(runs, list) and all(isinstance(row, dict) for row in runs), "runs must be objects")
    require(manifest.get("protocol_sha256") == digest(config), "protocol changed")
    phase = manifest.get("phase")
    require(phase in {"screen", "confirm", "calibrate"}, "invalid phase")
    experiment = manifest.get("experiment")
    if phase == "calibrate":
        require(experiment is None, "A/A calibration cannot select an experiment")
    else:
        require(experiment in config["experiments"], "unregistered experiment")
    require(set(manifest.get("artifacts", {})) == {"A", "B"}, "two artifact identities required")
    require(all(isinstance(x, str) and len(x) == 64 and all(c in "0123456789abcdef" for c in x) for x in manifest["artifacts"].values()), "invalid artifact digest")
    if phase == "calibrate":
        require(manifest["artifacts"]["A"] == manifest["artifacts"]["B"], "A/A calibration requires identical artifacts")
    expected = []
    for selection in config["selections"]:
        for cycle, block, arm, slot, seed in schedule(phase, selection_sentinel(config, experiment, selection)):
            expected.append((selection["scenario"], selection["profile"], cycle, block, arm, slot, seed))
    def key(row):
        return tuple(row.get(k) for k in ("scenario", "profile", "cycle", "block", "arm", "slot", "fixture_seed"))
    keys = [key(row) for row in runs]
    require(len(keys) == len(set(keys)), "duplicate independent session")
    require(set(keys).issubset(set(expected)), "unexpected/repaired/reordered block identity")
    # JSONL storage order may change, but recorded acquisition order may not.
    for row in runs:
        require(row.get("sequence") == expected.index(key(row)), "collection did not follow A/B/B/A order")
        require(row.get("protocol_sha256") == manifest["protocol_sha256"], "run protocol mismatch")
        require(row.get("artifact_sha256") == manifest["artifacts"][row["arm"]], "run artifact mismatch")
        selection = next(s for s in config["selections"] if s["scenario"] == row["scenario"] and s["profile"] == row["profile"])
        require(row.get("fixture_sha256") == digest(selection["fixture"]), "fixture changed")
        require(row.get("status") in {"ok", "functional_failure", "timeout", "harness_error"}, "unknown outcome")
        if row["status"] == "ok":
            require(row.get("profile_settings") == selection["profile_settings"], "run profile settings mismatch")
            require(row.get("identity_verified") is True, "served artifact identity unverified")
            require(row.get("diagnostics_enabled") is config["diagnostics_enabled"], "run instrumentation mode differs from protocol")
        require(row["status"] != "harness_error", "harness failure: rerun complete affected paired block with evidence in a new campaign")
        metrics = row.get("metrics", {})
        require(isinstance(metrics, dict), "metrics must be an object")
        for value in metrics.values():
            require(value is None or number(value), "metrics must be finite nonnegative session-level scalars")
    if any(row["status"] in {"functional_failure", "timeout"} for row in runs):
        return {"decision": "REJECT", "reasons": ["functional failure/timeout"], "failures": [row for row in runs if row["status"] != "ok"]}
    if set(keys) != set(expected):
        return {"decision": "INCONCLUSIVE", "reasons": ["missing independent sessions"]}
    report = {"decision": "INCONCLUSIVE", "reasons": [], "metrics": []}
    reject, uncertain, missing = [], [], []
    if any(r.get("profile_verified") is not True or r.get("environment_verified") is not True or r.get("fixture_verified") is not True for r in runs):
        missing.append("profile/environment/fixture validation unavailable")
    for selection in config["selections"]:
        rows = [r for r in runs if r["scenario"] == selection["scenario"] and r["profile"] == selection["profile"]]
        by_key = {(r["cycle"], r["block"], r["slot"]): r for r in rows}
        cycles = sorted({r["cycle"] for r in rows})
        slots = sorted({r["slot"] for r in rows})
        sentinel = selection_sentinel(config, experiment, selection)
        for metric, spec in selection["metrics"].items():
            label = f'{selection["scenario"]}/{selection["profile"]}/{metric}'
            if any(row.get("metrics", {}).get(metric) is None for row in rows):
                missing.append(label + ": unavailable metric")
                continue
            pairs = [[[(by_key[c, ablock, s]["metrics"][metric], by_key[c, bblock, s]["metrics"][metric]) for s in slots] for ablock, bblock in [(0, 1), (3, 2)]] for c in cycles]
            a = [v[0] for cycle in pairs for pair in cycle for v in pair]
            b = [v[1] for cycle in pairs for pair in cycle for v in pair]
            entry = {"endpoint": label, "classification": "SCREENED" if sentinel else "AFFECTED", "max": {"A": max(a), "B": max(b)}, "blocks": []}
            if "absolute_ceiling" in spec:
                entry["absolute_ceiling"] = spec["absolute_ceiling"]
                if any(value > spec["absolute_ceiling"] for value in b):
                    reject.append(label + ": absolute ceiling exceeded")
            for c in cycles:
                for block in range(4):
                    values = [by_key[c, block, s]["metrics"][metric] for s in slots]
                    entry["blocks"].append({"cycle": c, "block": block, "median": quantile(values, .5), "p95": quantile(values, .95)})
            primary = experiment is not None and config["experiments"][experiment]["primary"] == {"scenario": selection["scenario"], "profile": selection["profile"], "metric": metric}
            for q, name in [(.5, "median"), (.95, "p95")]:
                qa, qb = quantile(a, q), quantile(b, q)
                allowed = budget(spec["kind"], qa, q, spec.get("zero_absolute_budget"))
                def excess(left, right):
                    base = quantile(left, q)
                    permitted = budget(spec["kind"], base, q, spec.get("zero_absolute_budget"))
                    return None if permitted is None else quantile(right, q) - base - permitted
                interval = bootstrap(pairs, excess)
                entry[name] = {"A": qa, "B": qb, "improvement": qa - qb, "percent": None if qa == 0 else 100 * (qa - qb) / qa, "budget": allowed, "excess_ci95": interval}
                if allowed is None or interval is None:
                    missing.append(label + ": zero denominator without absolute budget")
                elif qb - qa > allowed:
                    reject.append(label + ": " + name + " exceeds regression budget")
                elif interval[1] > 0:
                    uncertain.append(label + ": " + name + " regression precision insufficient")
            if primary:
                improvements = []
                for cycle in pairs:
                    ca, cb = ([v[i] for pair in cycle for v in pair] for i in (0, 1))
                    baseline = quantile(ca, .5)
                    improvement = baseline - quantile(cb, .5)
                    improvements.append({"improvement": improvement, "target": target(spec["kind"], baseline)})
                entry["primary_cycles"] = improvements
                interval = bootstrap(pairs, lambda left, right: quantile(left, .5) - quantile(right, .5))
                entry["improvement_ci95"] = interval
                if any(c["improvement"] < c["target"] for c in improvements):
                    reject.append(label + ": practical target missed")
                if interval[0] <= 0:
                    uncertain.append(label + ": benefit interval includes zero")
            report["metrics"].append(entry)
    if phase == "calibrate":
        uncertain.append("A/A calibration cannot retain an optimization")
        report["calibration_precision_pass"] = not reject and not missing and not uncertain[:-1]
    elif manifest["artifacts"]["A"] == manifest["artifacts"]["B"]:
        uncertain.append("identical artifacts cannot retain")
    if phase == "screen":
        uncertain.append("screening cannot retain")
    if not config["frozen"]:
        uncertain.append("protocol is not frozen")
    required_matrix = {(f"S{i}", profile) for i in range(1, 9) for profile in PROFILES}
    if {(s["scenario"], s["profile"]) for s in config["selections"]} != required_matrix:
        uncertain.append("full S1-S8 desktop/constrained matrix unavailable")
    for selection in config["selections"]:
        required = {**REQUIRED_METRICS[selection["scenario"]], **{name: "resource" for name in FOOTPRINT_METRICS}}
        if any(selection["metrics"].get(name, {}).get("kind") != kind for name, kind in required.items()):
            uncertain.append(f'{selection["scenario"]}/{selection["profile"]}: required guard metric inventory incomplete')
    if manifest.get("final_integration") is True and experiment is not None and config["experiments"][experiment]["sentinels"]:
        uncertain.append("final integration requires confirmation for every selection")
    required_kinds = {"startup", "paint", "completion", "offline", "resource"}
    actual_kinds = {spec["kind"] for s in config["selections"] for spec in s["metrics"].values()}
    if not required_kinds.issubset(actual_kinds):
        uncertain.append("required readiness, interaction, completion, offline or resource guards unavailable")
    resource_metrics = {metric for s in config["selections"] for metric, spec in s["metrics"].items() if spec["kind"] == "resource"}
    if not {"total_transferred_bytes", "total_compiled_bytes", "peak_memory_bytes", "settled_memory_bytes", "cpu_ms"}.issubset(resource_metrics):
        uncertain.append("required total bytes, memory or CPU guards unavailable")
    # Evidence is a separate reviewed artifact, never inferred from collection success.
    required_evidence = {"calibration_pass", "instrumentation_overhead_pass", "correctness_pass", "platform_coverage_pass", "mechanism_pass", "original_baseline_pass", "visual_endpoint_validated", "full_matrix_coverage_pass", "sentinel_review_pass"}
    evidence = manifest.get("evidence", {})
    for item in sorted(required_evidence):
        if evidence.get(item) is not True:
            uncertain.append("missing evidence: " + item)
    # A functional failure returns above and always rejects. For latency data,
    # however, a missing observation or unverified run prevents a complete gate
    # even when another observed endpoint looks bad.
    report["decision"] = "INCONCLUSIVE" if missing else "REJECT" if reject else "INCONCLUSIVE" if uncertain else "RETAIN"
    report["reasons"] = reject + missing + uncertain
    return report


def artifact_digest(directory):
    directory = Path(directory).resolve(strict=True)
    require(directory.is_dir() and (directory / "index.html").is_file(), "artifact must contain index.html")
    entries = []
    for path in sorted(directory.rglob("*")):
        if path.is_file():
            entries.append((str(path.relative_to(directory)), hashlib.sha256(path.read_bytes()).hexdigest()))
    return digest(entries)


def baseline_protocol():
    """The deliberately small, frozen S1 observed-readiness procedure."""
    return {
        "schema_version": SCHEMA,
        "kind": "s1-observed-readiness-baseline-v3",
        "page_load_strategy": "none",
        "measurement": BASELINE_MEASUREMENT,
        "scenario": "S1",
        "profile": "desktop",
        "profile_settings": PROFILES["desktop"],
        "fixture": BASELINE_FIXTURE,
        "runs": BASELINE_RUNS,
        "poll_interval_ms": 50,
        "timeout_ms": 120000,
        "viewport": [1280, 800],
        "post_timing_checks": [
            "trusted WebDriver share click",
            "visible log in to share action",
            "visible 1 member roster row",
            "served artifact identity",
            "screenshot",
        ],
        "limitations": [
            "includes WebDriver polling and transport overhead",
            "is not LCP, exact presentation time, or field INP",
            "does not provide complete request or byte accounting",
        ],
    }


def summarize_baseline(rows, expected_runs=BASELINE_RUNS):
    statuses = {}
    failure_categories = {}
    timings = []
    for row in rows:
        status = row.get("status", "missing")
        statuses[status] = statuses.get(status, 0) + 1
        if status != "ok":
            category = row.get("failure_category", row.get("error", "unspecified"))
            failure_categories[category] = failure_categories.get(category, 0) + 1
            continue
        value = row.get("metrics", {}).get("navigation_to_usable_observed_ms")
        require(number(value), "successful baseline row omitted its observed-readiness timing")
        timings.append(value)
    statistics = None
    if timings:
        q1, median, q3 = (quantile(timings, q) for q in (.25, .5, .75))
        statistics = {
            "successful_timings_ms": timings,
            "minimum_ms": min(timings),
            "q1_ms": q1,
            "median_ms": median,
            "q3_ms": q3,
            "iqr_ms": q3 - q1,
            "maximum_ms": max(timings),
        }
    usable = (
        len(rows) == expected_runs
        and len(timings) == expected_runs
        and all(row.get("identity_verified") is True for row in rows)
        and all(row.get("functional_check", {}).get("share_menu_open") is True for row in rows)
    )
    return {
        "schema_version": SCHEMA,
        "measurement": BASELINE_MEASUREMENT,
        "expected_runs": expected_runs,
        "attempted_runs": len(rows),
        "successful_runs": len(timings),
        "status_counts": statuses,
        "failure_categories": failure_categories,
        "statistics": statistics,
        "usable_baseline": usable,
    }


def collect_baseline(args):
    artifact = Path(args.artifact).resolve(strict=True)
    identity = artifact_digest(artifact)
    output = Path(args.output).resolve()
    require(not output.exists(), "output exists; baseline collections are immutable")
    output.mkdir(parents=True)
    protocol = baseline_protocol()
    protocol_sha256 = digest(protocol)
    repo = Path(__file__).resolve().parents[2]
    source = subprocess.run(
        ["git", "rev-parse", "HEAD"], cwd=repo, capture_output=True, text=True, timeout=10
    )
    require(source.returncode == 0, "could not resolve baseline source SHA")
    source_sha = source.stdout.strip()
    require(len(source_sha) == 40, "git returned a malformed source SHA")
    write(output / "protocol.json", protocol)
    write(output / "environment.json", capture_environment())
    version = read(artifact / "version.json")
    manifest = {
        "schema_version": SCHEMA,
        "kind": protocol["kind"],
        "source_sha": source_sha,
        "protocol_sha256": protocol_sha256,
        "artifact_path": str(artifact),
        "artifact_sha256": identity,
        "artifact_version": version,
        "runner": RUNNER,
    }
    write(output / "manifest.json", manifest)

    rows = []
    with (output / "runs.jsonl").open("x") as raw:
        for sequence in range(BASELINE_RUNS):
            require(artifact_digest(artifact) == identity, "artifact changed before baseline run")
            request = {
                "schema_version": SCHEMA,
                "measurement": BASELINE_MEASUREMENT,
                "scenario": "S1",
                "profile": "desktop",
                "profile_settings": PROFILES["desktop"],
                "fixture": BASELINE_FIXTURE,
                "fixture_seed": f"baseline-{sequence}",
                "slot": sequence,
                "fixture_sha256": digest(BASELINE_FIXTURE),
            }
            request_path = output / f"request-{sequence:02d}.json"
            result_path = output / f"result-{sequence:02d}.json"
            write(request_path, request)
            env = dict(
                os.environ,
                TONK_PERF_TRACE="0",
                TONK_PERF_REQUEST=str(request_path),
                TONK_PERF_RESULT=str(result_path),
                TONK_UI_RELEASE_ARTIFACT=str(artifact),
            )
            with (output / f"runner-{sequence:02d}.log").open("x") as log:
                process = subprocess.Popen(
                    RUNNER, cwd=repo, env=env, stdout=log,
                    stderr=subprocess.STDOUT, start_new_session=True,
                )
                runner_timed_out = False
                try:
                    returncode = process.wait(timeout=600)
                except subprocess.TimeoutExpired:
                    runner_timed_out = True
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        returncode = process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        returncode = process.wait()
            if result_path.exists():
                try:
                    result = read(result_path)
                except (OSError, ValueError, TypeError):
                    result = {"status": "harness_error", "error": "runner emitted an unreadable result"}
                if not isinstance(result, dict):
                    result = {"status": "harness_error", "error": "runner result must be an object"}
            else:
                result = {"status": "harness_error", "error": "runner did not emit result"}
            if runner_timed_out:
                result = {"status": "harness_error", "error": "runner exceeded 600 seconds"}
            if returncode != 0:
                result = {**result, "runner_exit": returncode}
                if result.get("status") not in {"timeout", "functional_failure"}:
                    result["status"] = "harness_error"
            if result.get("status") == "ok" and (
                result.get("measurement") != BASELINE_MEASUREMENT
                or result.get("identity_verified") is not True
                or result.get("fixture_verified") is not True
                or result.get("profile_settings") != PROFILES["desktop"]
                or result.get("fixture_seed") != request["fixture_seed"]
                or result.get("slot") != sequence
                or result.get("fixture_sha256") != request["fixture_sha256"]
            ):
                result = {**result, "status": "harness_error", "error": "runner identity or fixture echo mismatched"}
            if artifact_digest(artifact) != identity:
                result = {**result, "status": "harness_error", "error": "artifact changed during baseline collection"}
            row = {
                **result,
                "sequence": sequence,
                "protocol_sha256": protocol_sha256,
                "artifact_sha256": identity,
            }
            rows.append(row)
            raw.write(json.dumps(row, sort_keys=True, allow_nan=False) + "\n")
            raw.flush()
            if row.get("status") == "harness_error":
                write(output / "summary.json", summarize_baseline(rows))
                raise Invalid(f"baseline harness failed; retained run {sequence} and its log")
    summary = summarize_baseline(rows)
    write(output / "summary.json", summary)
    print(json.dumps(summary, indent=2))
    return 0 if summary["usable_baseline"] else 2


def observed_comparison_summary(rows):
    blocks = []
    for block in range(4):
        arm = "A" if block in (0, 3) else "B"
        block_rows = [row for row in rows if row.get("block") == block]
        values = [
            row.get("metrics", {}).get("navigation_to_usable_observed_ms")
            for row in block_rows
            if row.get("status") == "ok"
        ]
        blocks.append({
            "block": block,
            "arm": arm,
            "successful_runs": len(values),
            "values_ms": values,
            "median_ms": quantile(values, .5) if values else None,
            "minimum_ms": min(values) if values else None,
            "maximum_ms": max(values) if values else None,
        })
    failures = [row for row in rows if row.get("status") != "ok"]
    pairs = []
    for a_block, b_block in ((0, 1), (3, 2)):
        a = blocks[a_block]["median_ms"]
        b = blocks[b_block]["median_ms"]
        if a is None or b is None:
            pairs.append({"A_block": a_block, "B_block": b_block, "available": False})
            continue
        improvement = a - b
        target_ms = max(100, .05 * a)
        pairs.append({
            "A_block": a_block,
            "B_block": b_block,
            "available": True,
            "A_median_ms": a,
            "B_median_ms": b,
            "improvement_ms": improvement,
            "improvement_percent": 100 * improvement / a,
            "target_ms": target_ms,
            "passes_practical_gate": improvement >= target_ms,
        })
    a_values = [value for block in (blocks[0], blocks[3]) for value in block["values_ms"]]
    b_values = [value for block in (blocks[1], blocks[2]) for value in block["values_ms"]]
    pooled = None
    if a_values and b_values:
        a_median, b_median = quantile(a_values, .5), quantile(b_values, .5)
        pooled = {
            "A_median_ms": a_median,
            "B_median_ms": b_median,
            "improvement_ms": a_median - b_median,
            "improvement_percent": 100 * (a_median - b_median) / a_median,
            "A_iqr_ms": quantile(a_values, .75) - quantile(a_values, .25),
            "B_iqr_ms": quantile(b_values, .75) - quantile(b_values, .25),
            "A_range_ms": [min(a_values), max(a_values)],
            "B_range_ms": [min(b_values), max(b_values)],
        }
    complete = len(rows) == 20 and all(block["successful_runs"] == 5 for block in blocks)
    if failures:
        decision = "REJECT"
        reason = "functional failure or timeout"
    elif not complete or not all(pair.get("available") for pair in pairs):
        decision = "INCONCLUSIVE"
        reason = "comparison is incomplete"
    elif all(pair["passes_practical_gate"] for pair in pairs):
        decision = "PROMISING"
        reason = "both adjacent block comparisons pass the practical gate; repeat required"
    elif all(
        0 <= pair["improvement_ms"] < pair["target_ms"] for pair in pairs
    ) or all(pair["improvement_ms"] <= 0 for pair in pairs):
        decision = "REJECT"
        reason = "both adjacent comparisons show a noise-level change or regression"
    else:
        decision = "INCONCLUSIVE"
        reason = "adjacent block comparisons disagree"
    return {
        "schema_version": SCHEMA,
        "measurement": BASELINE_MEASUREMENT,
        "schedule": "A/B/B/A with five fresh profiles per block",
        "attempted_runs": len(rows),
        "successful_runs": sum(block["successful_runs"] for block in blocks),
        "failures": failures,
        "blocks": blocks,
        "adjacent_comparisons": pairs,
        "pooled": pooled,
        "decision": decision,
        "reason": reason,
    }


def collect_observed_comparison(args):
    artifacts = {
        "A": Path(args.baseline).resolve(strict=True),
        "B": Path(args.candidate).resolve(strict=True),
    }
    identities = {arm: artifact_digest(path) for arm, path in artifacts.items()}
    require(identities["A"] != identities["B"], "comparison artifacts must differ")
    output = Path(args.output).resolve()
    require(not output.exists(), "output exists; observed comparisons are immutable")
    output.mkdir(parents=True)
    protocol = {
        **baseline_protocol(),
        "kind": "s1-observed-readiness-comparison-v3",
        "experiment": args.experiment,
        "schedule": ["A", "B", "B", "A"],
        "runs_per_block": 5,
        "practical_gate": "max(100 ms, 5% of adjacent A block median)",
    }
    protocol_sha256 = digest(protocol)
    repo = Path(__file__).resolve().parents[2]
    write(output / "protocol.json", protocol)
    write(output / "environment.json", capture_environment())
    manifest = {
        "schema_version": SCHEMA,
        "kind": protocol["kind"],
        "experiment": args.experiment,
        "protocol_sha256": protocol_sha256,
        "artifacts": identities,
        "artifact_paths": {arm: str(path) for arm, path in artifacts.items()},
        "artifact_versions": {arm: read(path / "version.json") for arm, path in artifacts.items()},
        "runner": RUNNER,
    }
    write(output / "manifest.json", manifest)

    schedule_rows = [
        (block, arm, slot)
        for block, arm in enumerate(("A", "B", "B", "A"))
        for slot in range(5)
    ]
    rows = []
    with (output / "runs.jsonl").open("x") as raw:
        for sequence, (block, arm, slot) in enumerate(schedule_rows):
            artifact = artifacts[arm]
            require(artifact_digest(artifact) == identities[arm], "artifact changed before comparison run")
            request = {
                "schema_version": SCHEMA,
                "measurement": BASELINE_MEASUREMENT,
                "scenario": "S1",
                "profile": "desktop",
                "profile_settings": PROFILES["desktop"],
                "fixture": BASELINE_FIXTURE,
                "fixture_seed": f"{args.experiment}-{block}-{slot}",
                "slot": slot,
                "fixture_sha256": digest(BASELINE_FIXTURE),
            }
            request_path = output / f"request-{sequence:02d}.json"
            result_path = output / f"result-{sequence:02d}.json"
            write(request_path, request)
            env = dict(
                os.environ,
                TONK_PERF_TRACE="0",
                TONK_PERF_REQUEST=str(request_path),
                TONK_PERF_RESULT=str(result_path),
                TONK_UI_RELEASE_ARTIFACT=str(artifact),
            )
            with (output / f"runner-{sequence:02d}.log").open("x") as log:
                process = subprocess.Popen(
                    RUNNER, cwd=repo, env=env, stdout=log,
                    stderr=subprocess.STDOUT, start_new_session=True,
                )
                runner_timed_out = False
                try:
                    returncode = process.wait(timeout=600)
                except subprocess.TimeoutExpired:
                    runner_timed_out = True
                    os.killpg(process.pid, signal.SIGTERM)
                    try:
                        returncode = process.wait(timeout=15)
                    except subprocess.TimeoutExpired:
                        os.killpg(process.pid, signal.SIGKILL)
                        returncode = process.wait()
            if result_path.exists():
                try:
                    result = read(result_path)
                except (OSError, ValueError, TypeError):
                    result = {"status": "harness_error", "error": "runner emitted an unreadable result"}
                if not isinstance(result, dict):
                    result = {"status": "harness_error", "error": "runner result must be an object"}
            else:
                result = {"status": "harness_error", "error": "runner did not emit result"}
            if runner_timed_out:
                result = {"status": "harness_error", "error": "runner exceeded 600 seconds"}
            if returncode != 0:
                result = {**result, "runner_exit": returncode}
                if result.get("status") not in {"timeout", "functional_failure"}:
                    result["status"] = "harness_error"
            if result.get("status") == "ok" and (
                result.get("measurement") != BASELINE_MEASUREMENT
                or result.get("identity_verified") is not True
                or result.get("fixture_verified") is not True
                or result.get("profile_settings") != PROFILES["desktop"]
                or result.get("fixture_seed") != request["fixture_seed"]
                or result.get("slot") != slot
                or result.get("fixture_sha256") != request["fixture_sha256"]
            ):
                result = {**result, "status": "harness_error", "error": "runner identity or fixture echo mismatched"}
            if artifact_digest(artifact) != identities[arm]:
                result = {**result, "status": "harness_error", "error": "artifact changed during comparison"}
            row = {
                **result,
                "sequence": sequence,
                "block": block,
                "arm": arm,
                "protocol_sha256": protocol_sha256,
                "artifact_sha256": identities[arm],
            }
            rows.append(row)
            raw.write(json.dumps(row, sort_keys=True, allow_nan=False) + "\n")
            raw.flush()
            if row.get("status") == "harness_error":
                write(output / "summary.json", observed_comparison_summary(rows))
                raise Invalid(f"comparison harness failed; retained run {sequence} and its log")
    summary = observed_comparison_summary(rows)
    write(output / "summary.json", summary)
    print(json.dumps({"decision": summary["decision"], "reason": summary["reason"]}, indent=2))
    return 0 if summary["decision"] == "PROMISING" else 2 if summary["decision"] == "REJECT" else 3


def collect(args):
    config = validate(read(args.config))
    require(config["frozen"], "freeze protocol after fixture and endpoint validation before collecting")
    phase = "calibrate" if args.command == "calibrate" else args.phase
    experiment = None if phase == "calibrate" else args.experiment
    require(phase == "calibrate" or experiment in config["experiments"], "experiment is not preregistered")
    artifacts = {"A": args.artifact, "B": args.artifact} if phase == "calibrate" else {"A": args.baseline, "B": args.candidate}
    identities = {arm: artifact_digest(path) for arm, path in artifacts.items()}
    output = Path(args.output).resolve()
    require(not output.exists(), "output exists; campaigns are immutable, use a new location")
    output.mkdir(parents=True)
    write(output / "protocol.json", config)
    manifest = {"schema_version": SCHEMA, "protocol_sha256": digest(config), "phase": phase, "experiment": experiment, "artifacts": identities, "artifact_paths": {k: str(Path(v).resolve()) for k, v in artifacts.items()}, "runner": RUNNER, "evidence": {}}
    write(output / "manifest.json", manifest)
    sequence = 0
    repo = Path(__file__).resolve().parents[2]
    with (output / "runs.jsonl").open("x") as raw:
        for selection in config["selections"]:
            for cycle, block, arm, slot, seed in schedule(phase, selection_sentinel(config, experiment, selection)):
                request = {"schema_version": SCHEMA, **selection, "fixture_seed": seed, "slot": slot, "fixture_sha256": digest(selection["fixture"])}
                require(artifact_digest(artifacts[arm]) == identities[arm], "artifact changed before collection")
                request_path = output / f"request-{sequence:04d}.json"
                result_path = output / f"result-{sequence:04d}.json"
                write(request_path, request)
                env = dict(os.environ, TONK_PERF_TRACE="1" if config["diagnostics_enabled"] else "0", TONK_PERF_REQUEST=str(request_path), TONK_PERF_RESULT=str(result_path), TONK_UI_RELEASE_ARTIFACT=manifest["artifact_paths"][arm])
                with (output / f"runner-{sequence:04d}.log").open("w") as log:
                    process = subprocess.Popen(RUNNER, cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
                    runner_timed_out = False
                    try:
                        returncode = process.wait(timeout=600)
                    except subprocess.TimeoutExpired:
                        runner_timed_out = True
                        os.killpg(process.pid, signal.SIGTERM)
                        try:
                            returncode = process.wait(timeout=15)
                        except subprocess.TimeoutExpired:
                            os.killpg(process.pid, signal.SIGKILL)
                            returncode = process.wait()
                if result_path.exists():
                    try:
                        result = read(result_path)
                    except (OSError, ValueError, TypeError):
                        result = {"status": "harness_error", "error": "runner emitted an unreadable result"}
                    if not isinstance(result, dict):
                        result = {"status": "harness_error", "error": "runner result must be an object"}
                else:
                    result = {"status": "harness_error", "error": "runner did not emit result"}
                if runner_timed_out:
                    result = {"status": "harness_error", "error": "runner exceeded 600s; process group terminated; distinguish infrastructure from application timeout before rerun"}
                if result.get("status") == "ok" and (result.get("fixture_seed") != seed or result.get("slot") != slot or result.get("fixture_sha256") != request["fixture_sha256"] or result.get("identity_verified") is not True or result.get("diagnostics_enabled") is not config["diagnostics_enabled"] or any(result.get(key) != request[key] for key in ("scenario", "profile", "profile_settings"))):
                    result = {**result, "status": "harness_error", "error": "runner identity/fixture echo missing or mismatched"}
                if result.get("status") not in {"ok", "functional_failure", "timeout", "harness_error"}:
                    result = {**result, "status": "harness_error", "error": "runner emitted an unknown outcome"}
                if returncode != 0 and result.get("status") not in {"functional_failure", "timeout"}:
                    result = {**result, "status": "harness_error", "runner_exit": returncode}
                elif returncode != 0:
                    result = {**result, "runner_exit": returncode}
                if artifact_digest(artifacts[arm]) != identities[arm]:
                    result = {**result, "status": "harness_error", "error": "artifact changed during collection"}
                row = {**result, "scenario": selection["scenario"], "profile": selection["profile"], "profile_settings": selection["profile_settings"], "diagnostics_enabled": config["diagnostics_enabled"], "cycle": cycle, "block": block, "arm": arm, "slot": slot, "fixture_seed": seed, "sequence": sequence, "protocol_sha256": manifest["protocol_sha256"], "artifact_sha256": identities[arm], "fixture_sha256": digest(selection["fixture"])}
                raw.write(json.dumps(row, sort_keys=True, allow_nan=False) + "\n")
                raw.flush()
                require(result.get("status") != "harness_error", f"harness failed; retained raw sample and runner log {sequence}")
                sequence += 1
    print(f"Collected {sequence} independent sessions in {output}; no optimization verdict. Run decide separately.")


def decide(directory):
    directory = Path(directory)
    config, manifest = read(directory / "protocol.json"), read(directory / "manifest.json")
    with (directory / "runs.jsonl").open() as stream:
        runs = [json.loads(line) for line in stream if line.strip()]
    result = analyze(config, manifest, runs)
    write(directory / "summary.json", {"protocol_sha256": digest(config), "metrics": result.get("metrics", []), "failures": result.get("failures", [])})
    decision = {k: v for k, v in result.items() if k not in {"metrics", "failures"}}
    write(directory / "decision.json", decision)
    print(json.dumps(decision, indent=2))
    return {"RETAIN": 0, "REJECT": 2, "INCONCLUSIVE": 3}[result["decision"]]


def main(argv=None):
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""Collection success is not retention. Raw runs.jsonl, summary.json and
decision.json are separate. decide exits 0 RETAIN, 2 REJECT, 3 INCONCLUSIVE;
configuration, identity and harness errors exit 1.

Examples (the provisional protocol must be validated and frozen first):
  python3 scripts/perf/ui-perf.py baseline --artifact /tmp/tonk-perf-b0 --output plans/ui-performance-results/s1-baseline-YYYYMMDD
  python3 scripts/perf/ui-perf.py compare-observed --baseline /tmp/tonk-perf-b0 --candidate /tmp/tonk-perf-e02 --experiment E02 --output plans/ui-performance-results/e02-screen-YYYYMMDD
  python3 scripts/perf/ui-perf.py validate --config plans/ui-performance-results/protocol.json
  python3 scripts/perf/ui-perf.py calibrate --artifact /tmp/tonk-perf-b0 --config plans/ui-performance-results/protocol.json --output /tmp/tonk-perf-aa
  python3 scripts/perf/ui-perf.py compare --baseline /tmp/tonk-perf-a --candidate /tmp/tonk-perf-b --experiment E01 --phase screen --config plans/ui-performance-results/protocol.json --output /tmp/tonk-perf-e01-screen
  python3 scripts/perf/ui-perf.py compare --baseline /tmp/tonk-perf-a --candidate /tmp/tonk-perf-b --experiment E01 --phase confirm --config plans/ui-performance-results/protocol.json --output /tmp/tonk-perf-e01-confirm
  python3 scripts/perf/ui-perf.py decide --results /tmp/tonk-perf-e01-confirm""",
    )
    commands = parser.add_subparsers(dest="command", required=True)
    baseline_parser = commands.add_parser(
        "baseline", help="collect the frozen ten-run S1 observed-readiness baseline"
    )
    baseline_parser.add_argument("--artifact", required=True)
    baseline_parser.add_argument("--output", required=True)
    observed_parser = commands.add_parser(
        "compare-observed", help="collect one fixed S1 A/B/B/A comparison"
    )
    observed_parser.add_argument("--baseline", required=True)
    observed_parser.add_argument("--candidate", required=True)
    observed_parser.add_argument("--experiment", required=True)
    observed_parser.add_argument("--output", required=True)
    validate_parser = commands.add_parser("validate", help="validate the explicit frozen-protocol schema")
    validate_parser.add_argument("--config", required=True)
    for command in ("calibrate", "compare"):
        child = commands.add_parser(command, help="collect isolated A/B/B/A sessions; never produce a retention verdict")
        if command == "calibrate":
            child.add_argument("--artifact", required=True)
        else:
            child.add_argument("--baseline", required=True)
            child.add_argument("--candidate", required=True)
            child.add_argument("--experiment", required=True)
            child.add_argument("--phase", choices=("screen", "confirm"), required=True)
        child.add_argument("--config", required=True)
        child.add_argument("--output", required=True)
    child = commands.add_parser("decide", help="calculate fixed-seed paired hierarchical intervals and gates")
    child.add_argument("--results", required=True)
    args = parser.parse_args(argv)
    try:
        if args.command == "baseline":
            return collect_baseline(args)
        if args.command == "compare-observed":
            return collect_observed_comparison(args)
        if args.command == "validate":
            config = validate(read(args.config))
            print(json.dumps({"valid": True, "frozen": config["frozen"], "protocol_sha256": digest(config)}))
            return 0
        if args.command == "decide":
            return decide(args.results)
        collect(args)
        return 0
    except (Invalid, OSError, ValueError, KeyError, TypeError) as error:
        print(f"INVALID: {error}", file=sys.stderr)
        return 1
