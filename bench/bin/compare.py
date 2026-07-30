#!/usr/bin/env python3
"""Compare two labelled agent-benchmark arms with a pre-registered gate."""

from __future__ import annotations

import argparse
import itertools
import json
import math
import random
import statistics
import sys
from pathlib import Path
from typing import Any


MIN_RUNS = 10
MIN_SUCCESSES = 8
MIN_MEDIAN_IMPROVEMENT_PCT = 25.0
MAX_SUCCESS_RATE_REGRESSION = 0.05
MAX_MEAN_OUTCOME_REGRESSION = 0.5
SIGNIFICANCE_ALPHA = 0.05
EXACT_COMBINATION_LIMIT = 500_000
MONTE_CARLO_SAMPLES = 100_000
BOOTSTRAP_SAMPLES = 20_000
RANDOM_SEED = 0x70A9


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Compare labelled Tonk agent-benchmark variants."
    )
    parser.add_argument("scenario")
    parser.add_argument("baseline")
    parser.add_argument("treatment")
    parser.add_argument(
        "--index",
        type=Path,
        help="JSONL index (default: <repo>/bench/runs/index.jsonl)",
    )
    parser.add_argument("--json", action="store_true", help="emit JSON")
    return parser.parse_args()


def load_records(path: Path, scenario: str, variant: str) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    with path.open() as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            try:
                record = json.loads(line)
            except json.JSONDecodeError as error:
                raise SystemExit(f"{path}:{line_number}: invalid JSON: {error}") from error
            if record.get("scenario") == scenario and record.get("variant") == variant:
                records.append(record)
    return records


def is_number(value: Any) -> bool:
    return isinstance(value, (int, float)) and not isinstance(value, bool)


def successful(record: dict[str, Any]) -> bool:
    verified = record.get("verified")
    correctness = (
        verified
        if isinstance(verified, bool)
        else is_number(record.get("outcome")) and record["outcome"] >= 7
    )
    return (
        correctness
        and is_number(record.get("first_write"))
    )


def mean(values: list[float]) -> float:
    return statistics.fmean(values)


def exact_or_monte_carlo_p(
    baseline: list[float], treatment: list[float]
) -> tuple[float, str, int]:
    """One-sided permutation p-value for baseline median > treatment median."""

    combined = baseline + treatment
    baseline_size = len(baseline)
    total_size = len(combined)
    observed = statistics.median(baseline) - statistics.median(treatment)
    combinations = math.comb(total_size, baseline_size)
    extreme = 0

    if combinations <= EXACT_COMBINATION_LIMIT:
        sample_count = combinations
        all_indices = set(range(total_size))
        for baseline_indices_tuple in itertools.combinations(
            range(total_size), baseline_size
        ):
            baseline_indices = set(baseline_indices_tuple)
            permuted_baseline = [combined[index] for index in baseline_indices]
            permuted_treatment = [
                combined[index] for index in all_indices - baseline_indices
            ]
            statistic = statistics.median(
                permuted_baseline
            ) - statistics.median(permuted_treatment)
            if statistic >= observed - 1e-12:
                extreme += 1
        return extreme / sample_count, "exact", sample_count

    rng = random.Random(RANDOM_SEED)
    sample_count = MONTE_CARLO_SAMPLES
    indices = list(range(total_size))
    for _ in range(sample_count):
        baseline_indices = set(rng.sample(indices, baseline_size))
        permuted_baseline = [combined[index] for index in baseline_indices]
        permuted_treatment = [
            combined[index] for index in indices if index not in baseline_indices
        ]
        statistic = statistics.median(
            permuted_baseline
        ) - statistics.median(permuted_treatment)
        if statistic >= observed - 1e-12:
            extreme += 1
    return (extreme + 1) / (sample_count + 1), "monte-carlo", sample_count


def cliff_improvement(baseline: list[float], treatment: list[float]) -> float:
    """Positive values mean treatment generally needs fewer commands."""

    wins = 0
    losses = 0
    for baseline_value in baseline:
        for treatment_value in treatment:
            if baseline_value > treatment_value:
                wins += 1
            elif baseline_value < treatment_value:
                losses += 1
    return (wins - losses) / (len(baseline) * len(treatment))


def bootstrap_improvement_interval(
    baseline: list[float], treatment: list[float]
) -> tuple[float | None, float | None]:
    rng = random.Random(RANDOM_SEED)
    improvements: list[float] = []
    for _ in range(BOOTSTRAP_SAMPLES):
        sampled_baseline = rng.choices(baseline, k=len(baseline))
        sampled_treatment = rng.choices(treatment, k=len(treatment))
        baseline_median = statistics.median(sampled_baseline)
        if baseline_median == 0:
            continue
        treatment_median = statistics.median(sampled_treatment)
        improvements.append(
            100.0 * (baseline_median - treatment_median) / baseline_median
        )
    if not improvements:
        return None, None
    improvements.sort()
    low = improvements[int(0.025 * (len(improvements) - 1))]
    high = improvements[int(0.975 * (len(improvements) - 1))]
    return low, high


def arm_summary(records: list[dict[str, Any]]) -> dict[str, Any]:
    successes = [record for record in records if successful(record)]
    writes = [float(record["first_write"]) for record in successes]
    outcomes = [
        float(record["outcome"])
        for record in records
        if is_number(record.get("outcome"))
    ]
    return {
        "runs": len(records),
        "successes": len(successes),
        "success_rate": len(successes) / len(records) if records else 0.0,
        "first_write_values": writes,
        "first_write_median": statistics.median(writes) if writes else None,
        "first_write_mean": mean(writes) if writes else None,
        "outcome_mean": mean(outcomes) if outcomes else None,
    }


def compare(
    baseline_records: list[dict[str, Any]],
    treatment_records: list[dict[str, Any]],
) -> dict[str, Any]:
    baseline = arm_summary(baseline_records)
    treatment = arm_summary(treatment_records)
    baseline_writes = baseline["first_write_values"]
    treatment_writes = treatment["first_write_values"]

    if not baseline_writes or not treatment_writes:
        raise SystemExit("both arms need at least one successful scored write")

    baseline_median = float(baseline["first_write_median"])
    treatment_median = float(treatment["first_write_median"])
    median_improvement_pct = (
        100.0 * (baseline_median - treatment_median) / baseline_median
        if baseline_median != 0
        else None
    )
    p_value, test_method, permutations = exact_or_monte_carlo_p(
        baseline_writes, treatment_writes
    )
    ci_low, ci_high = bootstrap_improvement_interval(
        baseline_writes, treatment_writes
    )

    checks = {
        "enough_runs": baseline["runs"] >= MIN_RUNS
        and treatment["runs"] >= MIN_RUNS,
        "enough_successes": baseline["successes"] >= MIN_SUCCESSES
        and treatment["successes"] >= MIN_SUCCESSES,
        "practical_effect": median_improvement_pct is not None
        and median_improvement_pct >= MIN_MEDIAN_IMPROVEMENT_PCT,
        "statistically_significant": p_value < SIGNIFICANCE_ALPHA,
        "success_guardrail": treatment["success_rate"]
        + MAX_SUCCESS_RATE_REGRESSION
        >= baseline["success_rate"],
        "outcome_guardrail": baseline["outcome_mean"] is not None
        and treatment["outcome_mean"] is not None
        and treatment["outcome_mean"] + MAX_MEAN_OUTCOME_REGRESSION
        >= baseline["outcome_mean"],
    }
    return {
        "baseline": baseline,
        "treatment": treatment,
        "effect": {
            "median_improvement_pct": median_improvement_pct,
            "bootstrap_95_pct": [ci_low, ci_high],
            "cliff_improvement": cliff_improvement(
                baseline_writes, treatment_writes
            ),
            "permutation_p_one_sided": p_value,
            "permutation_method": test_method,
            "permutations": permutations,
        },
        "checks": checks,
        "decision": "GRADUATE" if all(checks.values()) else "DO NOT GRADUATE",
    }


def render_text(
    scenario: str, baseline_name: str, treatment_name: str, result: dict[str, Any]
) -> str:
    baseline = result["baseline"]
    treatment = result["treatment"]
    effect = result["effect"]
    ci_low, ci_high = effect["bootstrap_95_pct"]

    def number(value: float | None, digits: int = 2) -> str:
        return "n/a" if value is None else f"{value:.{digits}f}"

    lines = [
        f"scenario: {scenario}",
        "",
        "arm | runs | successes | success rate | median first write | mean outcome",
        "--- | ---: | ---: | ---: | ---: | ---:",
        (
            f"{baseline_name} | {baseline['runs']} | {baseline['successes']} | "
            f"{baseline['success_rate']:.0%} | "
            f"{number(baseline['first_write_median'], 1)} | "
            f"{number(baseline['outcome_mean'])}"
        ),
        (
            f"{treatment_name} | {treatment['runs']} | {treatment['successes']} | "
            f"{treatment['success_rate']:.0%} | "
            f"{number(treatment['first_write_median'], 1)} | "
            f"{number(treatment['outcome_mean'])}"
        ),
        "",
        f"median improvement: {number(effect['median_improvement_pct'], 1)}%",
        f"bootstrap 95% interval: [{number(ci_low, 1)}%, {number(ci_high, 1)}%]",
        f"Cliff improvement: {number(effect['cliff_improvement'], 3)}",
        (
            "one-sided permutation p: "
            f"{effect['permutation_p_one_sided']:.6f} "
            f"({effect['permutation_method']}, {effect['permutations']} samples)"
        ),
        "",
    ]
    for check, passed in result["checks"].items():
        lines.append(f"{'PASS' if passed else 'FAIL'} {check}")
    lines.extend(["", f"decision: {result['decision']}"])
    return "\n".join(lines)


def main() -> None:
    args = parse_args()
    root = Path(__file__).resolve().parents[2]
    index = args.index or root / "bench" / "runs" / "index.jsonl"
    if not index.exists():
        raise SystemExit(f"no benchmark index at {index}")

    baseline_records = load_records(index, args.scenario, args.baseline)
    treatment_records = load_records(index, args.scenario, args.treatment)
    if not baseline_records:
        raise SystemExit(
            f"no {args.scenario!r} records for baseline {args.baseline!r}"
        )
    if not treatment_records:
        raise SystemExit(
            f"no {args.scenario!r} records for treatment {args.treatment!r}"
        )

    result = compare(baseline_records, treatment_records)
    if args.json:
        json.dump(result, sys.stdout, indent=2)
        print()
    else:
        print(render_text(args.scenario, args.baseline, args.treatment, result))


if __name__ == "__main__":
    main()
