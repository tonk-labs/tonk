import copy
import json
from pathlib import Path
import random
import tempfile
import unittest
from unittest.mock import Mock, patch

import ui_perf as perf


def protocol():
    result = {
        "schema_version": 1, "frozen": True, "diagnostics_enabled": True,
        "bootstrap": {"draws": 10000, "seed": 20260916, "quantile": "linear", "pairing": "cycle/block/seed/slot"},
        "selections": [{"scenario": "S1", "profile": "desktop", "profile_settings": perf.PROFILES["desktop"],
                        "fixture": {"recipe": "disposable anonymous", "selector": "visible nested welcome", "input_sequence": "trusted click", "dataset_sha256": "0" * 64, "cache_state": "empty HTTP/SW"},
                        "metrics": {"ready_ms": {"kind": "startup"}}}],
        "experiments": {"E01": {"hypothesis": "less work", "classification_reason": "startup only",
                                "primary": {"scenario": "S1", "profile": "desktop", "metric": "ready_ms"},
                                "affected": [{"scenario": "S1", "profile": "desktop"}], "sentinels": []}},
    }

    base = result["selections"][0]
    base["metrics"].update({"paint_ms": {"kind": "paint"}, "complete_ms": {"kind": "completion"}, "offline_ms": {"kind": "offline"}})
    for metric in ["total_transferred_bytes", "total_compiled_bytes", "peak_memory_bytes", "settled_memory_bytes", "cpu_ms"]:
        base["metrics"][metric] = {"kind": "resource"}
    result["selections"] = []
    for scenario in [f"S{i}" for i in range(1, 9)]:
        for profile in perf.PROFILES:
            selection = copy.deepcopy(base)
            selection.update(scenario=scenario, profile=profile, profile_settings=copy.deepcopy(perf.PROFILES[profile]))
            selection["metrics"].update({name: {"kind": kind} for name, kind in perf.REQUIRED_METRICS[scenario].items()})
            result["selections"].append(selection)
    result["experiments"]["E01"]["affected"] = [{"scenario": s["scenario"], "profile": s["profile"]} for s in result["selections"]]
    return result


def campaign(config=None, phase="confirm", a=lambda c, p, s: 1000, b=lambda c, p, s: 800):
    config = config or protocol()
    manifest = {"schema_version": 1, "protocol_sha256": perf.digest(config), "phase": phase, "experiment": None if phase == "calibrate" else "E01", "artifacts": {"A": "a" * 64, "B": "b" * 64},
                "evidence": dict.fromkeys(["calibration_pass", "instrumentation_overhead_pass", "correctness_pass", "platform_coverage_pass", "mechanism_pass", "original_baseline_pass", "visual_endpoint_validated", "full_matrix_coverage_pass", "sentinel_review_pass"], True)}
    if phase == "calibrate":
        manifest["artifacts"]["B"] = manifest["artifacts"]["A"]
    rows = []
    for selection in config["selections"]:
        for cycle, block, arm, slot, seed in perf.schedule(phase):
            value = (a if arm == "A" else b)(cycle, 0 if block < 2 else 1, slot)
            rows.append({"scenario": selection["scenario"], "profile": selection["profile"], "cycle": cycle, "block": block, "arm": arm, "slot": slot, "fixture_seed": seed, "sequence": len(rows),
                         "protocol_sha256": manifest["protocol_sha256"], "artifact_sha256": manifest["artifacts"][arm], "fixture_sha256": perf.digest(selection["fixture"]),
                         "diagnostics_enabled": config["diagnostics_enabled"], "profile_settings": selection["profile_settings"], "identity_verified": True, "profile_verified": True, "environment_verified": True, "fixture_verified": True, "status": "ok", "metrics": {metric: value if metric == "ready_ms" else 1000 for metric in selection["metrics"]}})
    return config, manifest, rows


class ObservedBaselineTests(unittest.TestCase):
    def test_protocol_freezes_the_revised_small_loop(self):
        protocol = perf.baseline_protocol()
        self.assertEqual(protocol["runs"], 10)
        self.assertEqual(protocol["poll_interval_ms"], 50)
        self.assertEqual(protocol["viewport"], [1280, 800])
        self.assertEqual(protocol["fixture"], perf.BASELINE_FIXTURE)
        self.assertEqual(protocol["measurement"], perf.BASELINE_MEASUREMENT)

    def test_summary_reports_linear_quartiles_and_range(self):
        rows = [
            {
                "status": "ok",
                "identity_verified": True,
                "functional_check": {"share_menu_open": True},
                "metrics": {"navigation_to_usable_observed_ms": value},
            }
            for value in range(100, 1100, 100)
        ]
        summary = perf.summarize_baseline(rows)
        self.assertTrue(summary["usable_baseline"])
        self.assertEqual(summary["statistics"]["minimum_ms"], 100)
        self.assertEqual(summary["statistics"]["q1_ms"], 325)
        self.assertEqual(summary["statistics"]["median_ms"], 550)
        self.assertEqual(summary["statistics"]["q3_ms"], 775)
        self.assertEqual(summary["statistics"]["iqr_ms"], 450)
        self.assertEqual(summary["statistics"]["maximum_ms"], 1000)

    def test_failures_remain_in_the_summary(self):
        rows = [{"status": "timeout", "failure_category": "observed_readiness_timeout"}]
        summary = perf.summarize_baseline(rows)
        self.assertFalse(summary["usable_baseline"])
        self.assertEqual(summary["successful_runs"], 0)
        self.assertEqual(summary["status_counts"], {"timeout": 1})
        self.assertEqual(summary["failure_categories"], {"observed_readiness_timeout": 1})

    def comparison_rows(self, medians=(1000, 800, 800, 1000)):
        return [
            {
                "block": block,
                "status": "ok",
                "metrics": {"navigation_to_usable_observed_ms": medians[block] + slot - 2},
            }
            for block in range(4)
            for slot in range(5)
        ]

    def test_observed_comparison_requires_both_adjacent_gates(self):
        summary = perf.observed_comparison_summary(self.comparison_rows())
        self.assertEqual(summary["decision"], "PROMISING")
        self.assertTrue(all(pair["passes_practical_gate"] for pair in summary["adjacent_comparisons"]))
        self.assertEqual(summary["pooled"]["improvement_ms"], 200)

        summary = perf.observed_comparison_summary(
            self.comparison_rows((1000, 800, 980, 1000))
        )
        self.assertEqual(summary["decision"], "INCONCLUSIVE")

    def test_observed_comparison_rejects_noise_and_failures(self):
        summary = perf.observed_comparison_summary(
            self.comparison_rows((1000, 950, 950, 1000))
        )
        self.assertEqual(summary["decision"], "REJECT")
        rows = self.comparison_rows()
        rows[7] = {"block": 1, "status": "timeout", "failure_category": "endpoint"}
        self.assertEqual(perf.observed_comparison_summary(rows)["decision"], "REJECT")


class StatisticsTests(unittest.TestCase):
    def test_calibration_requires_same_artifact(self):
        config, manifest, rows = campaign(phase="calibrate")
        manifest["artifacts"]["B"] = "b" * 64
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows)

    def test_calibration_cannot_select_experiment_sampling(self):
        config, manifest, rows = campaign(phase="calibrate")
        manifest["experiment"] = "E01"
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows)

    def test_primary_cannot_hide_in_extra_metadata(self):
        config = protocol()
        config["experiments"]["E01"]["primary"]["note"] = "ignored primary"
        with self.assertRaises(perf.Invalid):
            perf.validate(config)

    def test_linear_quantiles(self):
        self.assertEqual(perf.quantile([0, 10], .95), 9.5)
        self.assertEqual(perf.quantile([30, 10, 20, 40], .5), 25)

    def test_full_bootstrap_constant_shift(self):
        pairs = [[[(1000 + cycle * 100 + pair * 50 + slot, 800 + cycle * 100 + pair * 50 + slot) for slot in range(10)] for pair in range(2)] for cycle in range(3)]
        self.assertEqual(perf.bootstrap(pairs, lambda a, b: perf.quantile(a, .5) - perf.quantile(b, .5)), [200, 200])

    def test_paired_drift_cancels_and_is_deterministic(self):
        pairs = [[[(c * 10000 + p * 100 + s, c * 10000 + p * 100 + s) for s in range(10)] for p in range(2)] for c in range(3)]
        self.assertEqual(perf.bootstrap(pairs, lambda a, b: perf.quantile(a, .95) - perf.quantile(b, .95), draws=101), [0, 0])

    def test_schedule_freezes_blocks_counts_and_pair_seeds(self):
        screen = perf.schedule("screen")
        confirm = perf.schedule("confirm")
        self.assertEqual(len(screen), 20)
        self.assertEqual(len(confirm), 120)
        self.assertEqual([row[2] for row in screen], ["A"] * 5 + ["B"] * 10 + ["A"] * 5)
        for cycle in range(3):
            for slot in range(10):
                rows = [row for row in confirm if row[0] == cycle and row[3] == slot]
                self.assertEqual(rows[0][4], rows[1][4])
                self.assertEqual(rows[2][4], rows[3][4])
                self.assertNotEqual(rows[0][4], rows[2][4])

    def test_bootstrap_uses_the_frozen_hierarchy(self):
        pairs = [[[(100 * c + 10 * p + s, 100 * c + 10 * p + 2 * s) for s in range(10)] for p in range(2)] for c in range(3)]
        rng = random.Random(perf.SEED)
        left, right = [], []
        for _ in pairs:
            cycle = pairs[rng.randrange(len(pairs))]
            for _ in cycle:
                pair = cycle[rng.randrange(len(cycle))]
                for _ in pair:
                    a, b = pair[rng.randrange(len(pair))]
                    left.append(a)
                    right.append(b)
        expected = perf.quantile(left, .5) - perf.quantile(right, .5)
        self.assertEqual(perf.bootstrap(pairs, lambda a, b: perf.quantile(a, .5) - perf.quantile(b, .5), draws=1), [expected, expected])


class DecisionTests(unittest.TestCase):
    def setUp(self):
        # Full 10,000-draw arithmetic is tested above. Gate tests use the same
        # hierarchy with 100 draws to keep this deterministic suite focused.
        implementation = perf.bootstrap
        self.patcher = patch.object(perf, "bootstrap", lambda pairs, statistic: implementation(pairs, statistic, draws=100))
        self.patcher.start()
        self.addCleanup(self.patcher.stop)

    def test_partial_matrix_cannot_retain(self):
        config = protocol()
        config["selections"] = config["selections"][:1]
        config["experiments"]["E01"]["affected"] = [{"scenario": "S1", "profile": "desktop"}]
        self.assertEqual(perf.analyze(*campaign(config))["decision"], "INCONCLUSIVE")

    def test_missing_required_guard_cannot_retain(self):
        config = protocol()
        del config["selections"][0]["metrics"]["first_input_ms"]
        self.assertEqual(perf.analyze(*campaign(config))["decision"], "INCONCLUSIVE")

    def test_unverified_identity_is_invalid_even_in_incomplete_campaign(self):
        config, manifest, rows = campaign()
        rows[0]["identity_verified"] = False
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows[:1])

    def test_diagnostics_mode_mismatch_or_missing_is_invalid(self):
        for value in (False, None, 1):
            config, manifest, rows = campaign()
            rows[0]["diagnostics_enabled"] = value
            with self.assertRaises(perf.Invalid):
                perf.analyze(config, manifest, rows)

    def test_diagnostics_mode_must_be_explicit_boolean(self):
        for value in (None, 0, "false"):
            config = protocol()
            config["diagnostics_enabled"] = value
            with self.assertRaises(perf.Invalid):
                perf.validate(config)
        config = protocol()
        del config["diagnostics_enabled"]
        with self.assertRaises(perf.Invalid):
            perf.validate(config)

    def test_changed_profile_is_invalid(self):
        config, manifest, rows = campaign()
        rows[0]["profile_settings"] = {}
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows)

    def test_malformed_schema_exits_one(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "bad.json"
            path.write_text("[]")
            self.assertEqual(perf.main(["validate", "--config", str(path)]), 1)

    def test_malformed_runs_exit_one(self):
        config, manifest, rows = campaign()
        rows[0]["metrics"] = []
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            perf.write(root / "protocol.json", config)
            perf.write(root / "manifest.json", manifest)
            (root / "runs.jsonl").write_text("".join(json.dumps(row) + "\n" for row in rows))
            self.assertEqual(perf.main(["decide", "--results", str(root)]), 1)
            self.assertFalse((root / "decision.json").exists())

    def test_zero_test_runner_keeps_failed_raw_record(self):
        config = protocol()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "artifact"
            artifact.mkdir()
            (artifact / "index.html").write_text("fixture")
            perf.write(root / "config.json", config)
            with patch.dict(perf.os.environ, {"TONK_PERF_TRACE": "0"}), patch.object(perf.subprocess, "Popen") as popen:
                popen.return_value.wait.return_value = 0
                code = perf.main(["calibrate", "--artifact", str(artifact), "--config", str(root / "config.json"), "--output", str(root / "results")])
            self.assertEqual(code, 1)
            self.assertEqual(popen.call_args.kwargs["env"]["TONK_PERF_TRACE"], "1")
            rows = (root / "results" / "runs.jsonl").read_text().splitlines()
            self.assertEqual(len(rows), 1)
            self.assertEqual(json.loads(rows[0])["status"], "harness_error")
            self.assertFalse((root / "results" / "decision.json").exists())

    def test_malformed_runner_result_keeps_sanitized_raw_failure(self):
        for content, reason in [("{", "runner emitted an unreadable result"), ("[]\n", "runner result must be an object")]:
            with self.subTest(content=content):
                config = protocol()
                with tempfile.TemporaryDirectory() as directory:
                    root = Path(directory)
                    artifact = root / "artifact"
                    artifact.mkdir()
                    (artifact / "index.html").write_text("fixture")
                    perf.write(root / "config.json", config)

                    def start_runner(_command, **kwargs):
                        Path(kwargs["env"]["TONK_PERF_RESULT"]).write_text(content)
                        process = Mock()
                        process.wait.return_value = 0
                        return process

                    one_run = [(0, 0, "A", 0, "0:0:0")]
                    with patch.object(perf, "schedule", return_value=one_run), patch.object(perf.subprocess, "Popen", side_effect=start_runner):
                        code = perf.main(["calibrate", "--artifact", str(artifact), "--config", str(root / "config.json"), "--output", str(root / "results")])
                    self.assertEqual(code, 1)
                    self.assertEqual((root / "results" / "result-0000.json").read_text(), content)
                    rows = (root / "results" / "runs.jsonl").read_text().splitlines()
                    self.assertEqual(len(rows), 1)
                    row = json.loads(rows[0])
                    self.assertEqual(row["status"], "harness_error")
                    self.assertEqual(row["error"], reason)

    def test_runner_reported_timeout_remains_a_failed_sample(self):
        config = protocol()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            artifact = root / "artifact"
            artifact.mkdir()
            (artifact / "index.html").write_text("fixture")
            perf.write(root / "config.json", config)

            def start_runner(_command, **kwargs):
                perf.write(kwargs["env"]["TONK_PERF_RESULT"], {"status": "timeout"})
                process = Mock()
                process.wait.return_value = 1
                return process

            one_run = [(0, 0, "A", 0, "0:0:0")]
            with patch.object(perf, "schedule", return_value=one_run), patch.object(perf.subprocess, "Popen", side_effect=start_runner):
                code = perf.main(["calibrate", "--artifact", str(artifact), "--config", str(root / "config.json"), "--output", str(root / "results")])
            self.assertEqual(code, 0)
            rows = [json.loads(line) for line in (root / "results" / "runs.jsonl").read_text().splitlines()]
            self.assertEqual(len(rows), len(config["selections"]))
            self.assertTrue(all(row["status"] == "timeout" for row in rows))
            self.assertTrue(all(row["runner_exit"] == 1 for row in rows))
            self.assertEqual(rows[0]["profile_settings"], perf.PROFILES["desktop"])

    def test_clear_win(self):
        result = perf.analyze(*campaign())
        self.assertEqual(result["decision"], "RETAIN")
        self.assertEqual(len(result["metrics"][0]["blocks"]), 12)

    def test_small_or_no_win_rejected(self):
        for candidate in [999, 1000, 1100]:
            self.assertEqual(perf.analyze(*campaign(b=lambda c, p, s: candidate))["decision"], "REJECT")

    def test_each_cycle_must_pass(self):
        self.assertEqual(perf.analyze(*campaign(b=lambda c, p, s: 999 if c == 2 else 700))["decision"], "REJECT")

    def test_tail_only_regression(self):
        result = perf.analyze(*campaign(b=lambda c, p, s: 2000 if s == 9 else 800))
        self.assertEqual(result["decision"], "REJECT")
        self.assertTrue(any("p95" in r for r in result["reasons"]))

    def test_outlier_reported_in_maximum(self):
        result = perf.analyze(*campaign(b=lambda c, p, s: 5000 if (c, p, s) == (0, 0, 9) else 800))
        self.assertEqual(result["metrics"][0]["max"]["B"], 5000)
        self.assertEqual(result["decision"], "INCONCLUSIVE")

    def test_missing_and_null_metrics(self):
        for value in [None, "missing"]:
            config, manifest, rows = campaign()
            rows[2]["metrics"] = {} if value == "missing" else {"ready_ms": value}
            self.assertEqual(perf.analyze(config, manifest, rows)["decision"], "INCONCLUSIVE")

    def test_missing_metric_overrides_numeric_rejection(self):
        config, manifest, rows = campaign(b=lambda c, p, s: 1100)
        del rows[2]["metrics"]["paint_ms"]
        result = perf.analyze(config, manifest, rows)
        self.assertEqual(result["decision"], "INCONCLUSIVE")
        self.assertTrue(any("unavailable metric" in reason for reason in result["reasons"]))

    def test_missing_session(self):
        config, manifest, rows = campaign()
        self.assertEqual(perf.analyze(config, manifest, rows[:-1])["decision"], "INCONCLUSIVE")

    def test_functional_and_timeout_failures_survive(self):
        for status in ["functional_failure", "timeout"]:
            config, manifest, rows = campaign()
            rows[1]["status"] = status
            for field in ["metrics", "profile_settings", "diagnostics_enabled", "identity_verified", "profile_verified", "environment_verified", "fixture_verified"]:
                rows[1].pop(field, None)
            result = perf.analyze(config, manifest, rows)
            self.assertEqual(result["decision"], "REJECT")
            self.assertEqual(result["failures"][0]["status"], status)

    def test_harness_failure_invalidates(self):
        config, manifest, rows = campaign()
        rows[1]["status"] = "harness_error"
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows)

    def test_changed_fixture_artifact_protocol_invalidates(self):
        for field in ["fixture_sha256", "artifact_sha256", "protocol_sha256"]:
            config, manifest, rows = campaign()
            rows[1][field] = "changed"
            with self.assertRaises(perf.Invalid):
                perf.analyze(config, manifest, rows)

    def test_storage_reordering_is_harmless_collection_reordering_invalid(self):
        config, manifest, rows = campaign()
        random.Random(10).shuffle(rows)
        self.assertEqual(perf.analyze(config, manifest, rows)["decision"], "RETAIN")
        rows[0]["sequence"], rows[1]["sequence"] = rows[1]["sequence"], rows[0]["sequence"]
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows)

    def test_correlated_events_cannot_inflate_sessions(self):
        config, manifest, rows = campaign()
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows + [rows[0]])
        rows[0]["metrics"]["ready_ms"] = [800] * 100
        with self.assertRaises(perf.Invalid):
            perf.analyze(config, manifest, rows)

    def test_screening_never_retains(self):
        self.assertEqual(perf.analyze(*campaign(phase="screen"))["decision"], "INCONCLUSIVE")

    def test_identical_artifacts_never_retain(self):
        config, manifest, rows = campaign()
        manifest["artifacts"]["B"] = manifest["artifacts"]["A"]
        for row in rows:
            row["artifact_sha256"] = manifest["artifacts"][row["arm"]]
        self.assertEqual(perf.analyze(config, manifest, rows)["decision"], "INCONCLUSIVE")

    def test_aa_calibration_reports_precision_but_never_retains(self):
        result = perf.analyze(*campaign(phase="calibrate", b=lambda c, p, s: 1000))
        self.assertEqual(result["decision"], "INCONCLUSIVE")
        self.assertTrue(result["calibration_precision_pass"])

    def test_evidence_and_environment_are_required(self):
        config, manifest, rows = campaign()
        manifest["evidence"] = {}
        rows[0]["environment_verified"] = False
        self.assertEqual(perf.analyze(config, manifest, rows)["decision"], "INCONCLUSIVE")

    def test_zero_resource_baseline_needs_absolute_budget(self):
        config, manifest, rows = campaign()
        config["selections"][0]["metrics"]["memory"] = {"kind": "resource"}
        manifest["protocol_sha256"] = perf.digest(config)
        for row in rows:
            row["protocol_sha256"] = manifest["protocol_sha256"]
            row["metrics"]["memory"] = 0
        self.assertEqual(perf.analyze(config, manifest, rows)["decision"], "INCONCLUSIVE")
        config["selections"][0]["metrics"]["memory"]["zero_absolute_budget"] = 0
        manifest["protocol_sha256"] = perf.digest(config)
        for row in rows:
            row["protocol_sha256"] = manifest["protocol_sha256"]
        self.assertEqual(perf.analyze(config, manifest, rows)["decision"], "RETAIN")

    def test_preregistered_absolute_ceiling_rejects_single_candidate_breach(self):
        config = protocol()
        config["selections"][0]["metrics"]["ready_ms"]["absolute_ceiling"] = 850
        config, manifest, rows = campaign(config)
        candidate = next(row for row in rows if row["scenario"] == "S1" and row["profile"] == "desktop" and row["arm"] == "B")
        candidate["metrics"]["ready_ms"] = 851
        result = perf.analyze(config, manifest, rows)
        self.assertEqual(result["decision"], "REJECT")
        self.assertTrue(any("absolute ceiling exceeded" in reason for reason in result["reasons"]))

    def test_absolute_ceiling_must_be_nonnegative_and_finite(self):
        for ceiling in [-1, float("inf"), True]:
            config = protocol()
            config["selections"][0]["metrics"]["ready_ms"]["absolute_ceiling"] = ceiling
            with self.assertRaises(perf.Invalid):
                perf.validate(config)

    def test_invalid_settings_and_classification(self):
        config = protocol()
        config["selections"][0]["profile_settings"] = {}
        with self.assertRaises(perf.Invalid):
            perf.validate(config)
        config = protocol()
        config["experiments"]["E01"]["affected"] = []
        with self.assertRaises(perf.Invalid):
            perf.validate(config)

    def test_decision_exit_codes_and_separate_artifacts(self):
        config, manifest, rows = campaign()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            perf.write(root / "protocol.json", config)
            perf.write(root / "manifest.json", manifest)
            (root / "runs.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
            self.assertEqual(perf.decide(root), 0)
            self.assertTrue((root / "summary.json").exists())
            self.assertNotIn("metrics", perf.read(root / "decision.json"))


if __name__ == "__main__":
    unittest.main()
