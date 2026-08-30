#!/usr/bin/env python3
"""Compare Tonk's stock and pooled Wasm runners on identical archives."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import signal
import statistics
import subprocess
import sys
import time
from typing import Any


RUNNER_ENV = "CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER"
EXPERIMENTAL_JSON_ENV = "NEXTEST_EXPERIMENTAL_LIBTEST_JSON"
DEFAULT_TIMEOUT_SECONDS = 90 * 60
SUMMARY_RE = re.compile(r"Summary\s+\[[^]]+\]\s+\d+\s+tests?\s+run:\s*(.*)")
COUNT_RE = re.compile(r"(\d+)\s+(passed|skipped|failed)")
ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


class BenchmarkError(RuntimeError):
    pass


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--profiles",
        choices=("debug", "release"),
        nargs="+",
        default=("debug", "release"),
        help="Wasm archive profiles to compare (default: debug release)",
    )
    parser.add_argument("--runs", type=int, default=3, help="Runs per runner and profile")
    parser.add_argument("--output", type=Path, required=True, help="JSON evidence path")
    args = parser.parse_args()
    if args.runs < 1:
        parser.error("--runs must be at least 1")
    return args


def resolve_executable(value: str | None, name: str) -> str:
    candidate = value or shutil.which(name)
    if not candidate:
        raise BenchmarkError(f"could not find {name} on PATH")
    path = Path(candidate).expanduser().resolve()
    if not path.is_file() or not os.access(path, os.X_OK):
        raise BenchmarkError(f"{name} is not an executable file: {path}")
    return str(path)


def log_path_for(output: Path, label: str) -> Path:
    return output.parent / f"{output.stem}-{label}.log"


def stop_process_group(
    process: subprocess.Popen[str], *, grace_seconds: float = 5.0
) -> str:
    """Stop a benchmark command and every descendant that inherited its group."""
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except ProcessLookupError:
        pass
    try:
        output, _ = process.communicate(timeout=grace_seconds)
        return output or ""
    except subprocess.TimeoutExpired as error:
        captured = error.stdout or ""
        if isinstance(captured, bytes):
            captured = captured.decode(errors="replace")
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        try:
            output, _ = process.communicate(timeout=grace_seconds)
        except subprocess.TimeoutExpired:
            if process.stdout is not None:
                process.stdout.close()
            process.wait(timeout=grace_seconds)
            return captured
        return output or captured


def run_logged(
    command: list[str],
    log_path: Path,
    *,
    env: dict[str, str] | None = None,
    timeout: int | None = None,
) -> dict[str, Any]:
    started = time.monotonic()
    timed_out = False
    interrupted = False
    exit_status: int | None
    output = ""
    process = subprocess.Popen(
        command,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    try:
        output, _ = process.communicate(timeout=timeout)
        exit_status = process.returncode
    except subprocess.TimeoutExpired:
        timed_out = True
        exit_status = None
        output = stop_process_group(process)
    except KeyboardInterrupt:
        interrupted = True
        exit_status = None
        output = stop_process_group(process)
    duration = time.monotonic() - started

    log_path.parent.mkdir(parents=True, exist_ok=True)
    log_path.write_text(
        f"$ {shlex.join(command)}\n"
        f"exit_status={exit_status}\n"
        f"timed_out={str(timed_out).lower()}\n"
        f"duration_seconds={duration:.6f}\n\n"
        f"{output}"
    )
    if interrupted:
        raise KeyboardInterrupt
    return {
        "command": command,
        "duration_seconds": duration,
        "exit_status": exit_status,
        "timed_out": timed_out,
        "output": output,
        "log": str(log_path.resolve()),
    }


def require_success(result: dict[str, Any], operation: str) -> None:
    if result["timed_out"]:
        raise BenchmarkError(f"{operation} timed out; see {result['log']}")
    if result["exit_status"] != 0:
        raise BenchmarkError(
            f"{operation} exited with status {result['exit_status']}; see {result['log']}"
        )


def create_gc_root(
    store_path: Path, root_path: Path, output: Path, label: str
) -> dict[str, str]:
    result = run_logged(
        [
            "nix-store",
            "--add-root",
            str(root_path),
            "--indirect",
            "--realise",
            str(store_path),
        ],
        log_path_for(output, f"{label}-gc-root"),
    )
    require_success(result, f"rooting {label} for benchmark")
    return {
        "label": label,
        "path": str(root_path),
        "store_path": str(store_path),
        "log": result["log"],
    }


def archive_identity(archive: Path) -> dict[str, Any]:
    digest = hashlib.sha256()
    with archive.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return {
        "path": str(archive.resolve()),
        "sha256": digest.hexdigest(),
        "size_bytes": archive.stat().st_size,
    }


def build_archive(profile: str, output: Path) -> tuple[Path, dict[str, Any]]:
    package = f"tests-web-{profile}"
    result = run_logged(
        [
            "nix",
            "build",
            "--accept-flake-config",
            f".#{package}",
            "--no-link",
            "--print-out-paths",
        ],
        log_path_for(output, f"{profile}-build"),
    )
    require_success(result, f"building {package}")
    store_paths = [Path(line.strip()) for line in result["output"].splitlines() if line.startswith("/")]
    if not store_paths:
        raise BenchmarkError(f"nix did not print an output path for {package}; see {result['log']}")
    archive = store_paths[-1] / f"{package}.tar.zst"
    if not archive.is_file():
        raise BenchmarkError(f"archive does not exist: {archive}")
    return archive, result


def decode_json_lines(output: str) -> list[dict[str, Any]]:
    messages: list[dict[str, Any]] = []
    for line in output.splitlines():
        stripped = line.strip()
        if not stripped.startswith("{"):
            continue
        try:
            message = json.loads(stripped)
        except json.JSONDecodeError:
            continue
        if isinstance(message, dict):
            messages.append(message)
    return messages


def list_inventory(
    archive: Path, profile: str, stock_runner: str, output: Path
) -> tuple[dict[str, Any], dict[str, Any]]:
    env = os.environ.copy()
    env[RUNNER_ENV] = stock_runner
    result = run_logged(
        [
            "cargo",
            "nextest",
            "list",
            "--workspace-remap",
            "./",
            "--archive-file",
            str(archive),
            "--message-format",
            "json",
            "--color",
            "never",
        ],
        log_path_for(output, f"{profile}-inventory"),
        env=env,
    )
    require_success(result, f"listing {profile} archive")

    listings = [message for message in decode_json_lines(result["output"]) if "rust-suites" in message]
    if len(listings) != 1:
        raise BenchmarkError(
            f"expected one nextest inventory for {profile}, found {len(listings)}; see {result['log']}"
        )
    listing = listings[0]
    tests: list[dict[str, Any]] = []
    for suite_key, suite in listing["rust-suites"].items():
        binary_id = suite.get("binary-id", suite_key)
        for test_name, testcase in suite.get("testcases", {}).items():
            tests.append(
                {
                    "binary_id": binary_id,
                    "name": test_name,
                    "kind": testcase.get("kind", "test"),
                    "ignored": bool(testcase.get("ignored", False)),
                }
            )
    tests.sort(key=lambda test: (test["binary_id"], test["name"]))
    declared_count = listing.get("test-count")
    if declared_count != len(tests):
        raise BenchmarkError(
            f"nextest declared {declared_count} tests but listed {len(tests)} for {profile}"
        )
    encoded = json.dumps(tests, sort_keys=True, separators=(",", ":")).encode()
    return {
        "count": len(tests),
        "sha256": hashlib.sha256(encoded).hexdigest(),
        "tests": tests,
        "log": result["log"],
    }, result


def parse_run_output(output: str, inventory_count: int) -> dict[str, Any]:
    clean_output = ANSI_RE.sub("", output)
    summary = None
    counts: dict[str, int] | None = None
    for line in clean_output.splitlines():
        match = SUMMARY_RE.search(line)
        if match:
            summary = line.strip()
            counts = {"passed": 0, "skipped": 0, "failed": 0}
            for value, name in COUNT_RE.findall(match.group(1)):
                counts[name] = int(value)

    outcomes: dict[str, str] = {}
    terminal_events = 0
    for message in decode_json_lines(clean_output):
        if message.get("type") != "test":
            continue
        event = message.get("event")
        name = message.get("name")
        if event in {"ok", "failed", "ignored", "timeout"} and isinstance(name, str):
            terminal_events += 1
            outcomes[name] = event
    retries = max(0, terminal_events - len(outcomes))
    outcome_digest = hashlib.sha256(
        json.dumps(outcomes, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()

    inventory_matches = counts is not None and sum(counts.values()) == inventory_count
    return {
        "summary": summary,
        "counts": counts,
        "retries": retries,
        "observed_test_count": len(outcomes),
        "outcome_sha256": outcome_digest,
        "inventory_matches": inventory_matches,
    }


def run_archive(
    archive: Path,
    profile: str,
    runner_name: str,
    runner_path: str,
    run_number: int,
    inventory_count: int,
    output: Path,
    timeout_seconds: int,
    base_env: dict[str, str],
) -> dict[str, Any]:
    env = base_env.copy()
    env[RUNNER_ENV] = runner_path
    env[EXPERIMENTAL_JSON_ENV] = "1"
    result = run_logged(
        [
            "cargo",
            "nextest",
            "run",
            "--workspace-remap",
            "./",
            "--archive-file",
            str(archive),
            "--test-threads",
            "4",
            "--retries",
            "0",
            "--message-format",
            "libtest-json-plus",
            "--color",
            "never",
        ],
        log_path_for(output, f"{profile}-{runner_name}-{run_number}"),
        env=env,
        timeout=timeout_seconds,
    )
    parsed = parse_run_output(result.pop("output"), inventory_count)
    result.update(parsed)
    result["run"] = run_number
    return result


def compare_profile(profile: dict[str, Any]) -> dict[str, Any]:
    failures: list[str] = []
    inventory_count = profile["inventory"]["count"]
    runners = profile["runners"]
    stock_runs = runners["stock"]["runs"]
    pool_runs = runners["pool"]["runs"]
    baseline = stock_runs[0] if stock_runs else None

    for runner_name, runner in runners.items():
        durations = [run["duration_seconds"] for run in runner["runs"]]
        runner["durations_seconds"] = durations
        runner["median_duration_seconds"] = statistics.median(durations) if durations else None
        for run in runner["runs"]:
            label = f"{runner_name} run {run['run']}"
            if run["timed_out"]:
                failures.append(f"{label} timed out")
            if run["exit_status"] != 0:
                failures.append(f"{label} exit status was {run['exit_status']}")
            if run["counts"] is None:
                failures.append(f"{label} had no parseable nextest summary counts")
            elif sum(run["counts"].values()) != inventory_count:
                failures.append(
                    f"{label} counts total {sum(run['counts'].values())}, expected inventory {inventory_count}"
                )
            if run["observed_test_count"] != inventory_count:
                failures.append(
                    f"{label} observed {run['observed_test_count']} terminal test outcomes, "
                    f"expected inventory {inventory_count}"
                )
            if run["retries"] != 0:
                failures.append(f"{label} reported {run['retries']} retries")
            if baseline is not None and run["counts"] != baseline["counts"]:
                failures.append(f"{label} counts differ from stock baseline")
            if baseline is not None and run["outcome_sha256"] != baseline["outcome_sha256"]:
                failures.append(f"{label} observed test outcomes differ from stock baseline")

    stock_median = runners["stock"]["median_duration_seconds"]
    pool_median = runners["pool"]["median_duration_seconds"]
    ratio = pool_median / stock_median if stock_median and pool_median is not None else None
    if ratio is None:
        failures.append("could not calculate pooled-to-stock median ratio")
    elif ratio > 0.5:
        failures.append(f"pooled median is {ratio:.3f} of stock, expected no more than 0.500")

    return {
        "passed": not failures,
        "failures": failures,
        "pooled_to_stock_median_ratio": ratio,
    }


def write_evidence(output: Path, evidence: dict[str, Any]) -> None:
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(evidence, indent=2, sort_keys=True) + "\n")


def main() -> int:
    args = parse_args()
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    timeout_seconds = int(os.environ.get("WASM_RUNNER_BENCHMARK_TIMEOUT", DEFAULT_TIMEOUT_SECONDS))
    evidence: dict[str, Any] = {
        "schema_version": 1,
        "generated_at_unix_seconds": int(time.time()),
        "configuration": {
            "profiles": args.profiles,
            "runs": args.runs,
            "test_threads": 4,
            "retries": 0,
            "timeout_seconds": timeout_seconds,
        },
        "profiles": {},
        "cleanup": {},
        "gc_roots": [],
        "failures": [],
        "passed": False,
    }
    pool_state_dir = output.parent / f"{output.stem}-wbg-pool-{os.getpid()}"
    pool_gc_root = output.parent / f".{output.stem}-wbg-pool-gc-root-{os.getpid()}"
    pool_env = os.environ.copy()
    pool_env["WBG_POOL_DIR"] = str(pool_state_dir)
    evidence["pool_lifecycle"] = {
        "state_dir": str(pool_state_dir),
        "log": str((pool_state_dir / "daemon.log").resolve()),
        "daemon_starts": 0,
        "gc_root": str(pool_gc_root),
    }

    pool_runner: str | None = None
    gc_root_paths: list[Path] = []
    interrupted = False
    try:
        stock_runner = resolve_executable(
            os.environ.get("WBG_POOL_FALLBACK_RUNNER"), "wasm-bindgen-test-runner"
        )
        pool_runner = resolve_executable(None, "wbg-pool")
        evidence["runners"] = {"stock": stock_runner, "pool": pool_runner}

        for label, executable, root_path in (
            ("stock-runner", stock_runner, output.parent / f".{output.stem}-stock-gc-root-{os.getpid()}"),
            ("pool-runner", pool_runner, pool_gc_root),
        ):
            gc_root_paths.append(root_path)
            evidence["gc_roots"].append(
                create_gc_root(Path(executable).parent.parent, root_path, output, label)
            )

        before = run_logged(
            [pool_runner, "daemon", "--stop"],
            log_path_for(output, "daemon-before"),
            env=pool_env,
        )
        evidence["cleanup"]["before"] = {key: value for key, value in before.items() if key != "output"}
        require_success(before, "stopping wbg-pool before benchmark")

        archives: dict[str, Path] = {}
        for profile_name in args.profiles:
            archive, build_result = build_archive(profile_name, output)
            archives[profile_name] = archive
            archive_gc_root = (
                output.parent
                / f".{output.stem}-{profile_name}-archive-gc-root-{os.getpid()}"
            )
            gc_root_paths.append(archive_gc_root)
            evidence["gc_roots"].append(
                create_gc_root(archive.parent, archive_gc_root, output, f"{profile_name}-archive")
            )
            inventory, inventory_result = list_inventory(
                archive, profile_name, stock_runner, output
            )
            profile: dict[str, Any] = {
                "archive": archive_identity(archive),
                "build_log": build_result["log"],
                "inventory": inventory,
                "inventory_log": inventory_result["log"],
                "runners": {
                    "stock": {"path": stock_runner, "runs": []},
                    "pool": {"path": pool_runner, "runs": []},
                },
            }
            evidence["profiles"][profile_name] = profile

        # Keep pooled runs contiguous. Otherwise a long stock run between
        # profiles can exceed wbg-pool's idle timeout and turn one benchmark
        # job into multiple daemon/browser lifetimes.
        for runner_name, runner_path, runner_env in (
            ("stock", stock_runner, os.environ.copy()),
            ("pool", pool_runner, pool_env),
        ):
            for profile_name in args.profiles:
                profile = evidence["profiles"][profile_name]
                for run_number in range(1, args.runs + 1):
                    run = run_archive(
                        archives[profile_name],
                        profile_name,
                        runner_name,
                        runner_path,
                        run_number,
                        profile["inventory"]["count"],
                        output,
                        timeout_seconds,
                        runner_env,
                    )
                    profile["runners"][runner_name]["runs"].append(run)

        for profile in evidence["profiles"].values():
            profile["comparison"] = compare_profile(profile)
    except (BenchmarkError, OSError, ValueError) as error:
        evidence["failures"].append(str(error))
    except KeyboardInterrupt:
        interrupted = True
        evidence["failures"].append("benchmark interrupted")
    finally:
        if pool_runner is not None:
            try:
                after = run_logged(
                    [pool_runner, "daemon", "--stop"],
                    log_path_for(output, "daemon-after"),
                    env=pool_env,
                )
            except OSError as error:
                evidence["cleanup"]["after"] = {"error": str(error)}
                evidence["failures"].append(
                    f"stopping wbg-pool after benchmark failed: {error}"
                )
            else:
                evidence["cleanup"]["after"] = {
                    key: value for key, value in after.items() if key != "output"
                }
                if after["timed_out"] or after["exit_status"] != 0:
                    evidence["failures"].append(
                        f"stopping wbg-pool after benchmark failed; see {after['log']}"
                    )
        for gc_root_path in reversed(gc_root_paths):
            if gc_root_path.is_symlink():
                gc_root_path.unlink()
            elif gc_root_path.exists():
                evidence["failures"].append(
                    f"refusing to remove unexpected non-symlink GC root: {gc_root_path}"
                )

    daemon_log = pool_state_dir / "daemon.log"
    if daemon_log.is_file():
        daemon_starts = sum(
            "wbg-pool daemon listening on " in line for line in daemon_log.read_text().splitlines()
        )
        evidence["pool_lifecycle"]["daemon_starts"] = daemon_starts
    pool_runs = sum(
        len(profile.get("runners", {}).get("pool", {}).get("runs", []))
        for profile in evidence["profiles"].values()
    )
    if pool_runs and evidence["pool_lifecycle"]["daemon_starts"] != 1:
        evidence["failures"].append(
            "wbg-pool started "
            f"{evidence['pool_lifecycle']['daemon_starts']} daemons during {pool_runs} pooled runs; "
            f"see {evidence['pool_lifecycle']['log']}"
        )

    profiles_pass = bool(evidence["profiles"]) and all(
        profile.get("comparison", {}).get("passed", False)
        for profile in evidence["profiles"].values()
    )
    evidence["passed"] = profiles_pass and not evidence["failures"]
    write_evidence(output, evidence)
    print(f"wrote benchmark evidence to {output}")
    if interrupted:
        return 130
    if evidence["passed"]:
        return 0
    for failure in evidence["failures"]:
        print(f"benchmark setup failure: {failure}", file=sys.stderr)
    for profile_name, profile in evidence["profiles"].items():
        for failure in profile.get("comparison", {}).get("failures", []):
            print(f"{profile_name}: {failure}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
