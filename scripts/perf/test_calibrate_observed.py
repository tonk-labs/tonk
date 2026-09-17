import unittest

import calibrate_observed as calibration


class CalibrationTests(unittest.TestCase):
    def rows(self, differences):
        return [{"pair": pair, "arm": arm, "order": order, "sequence": sequence,
                 "status": "ok", "browser": {"product": "fixed"},
                 "metrics": {calibration.METRIC: 1000 + (differences[pair] if arm == "B" else 0)}}
                for sequence, (pair, arm, order) in enumerate(calibration.schedule(len(differences), 42))]

    def test_schedule_is_reproducible_balanced_and_adjacent(self):
        schedule = calibration.schedule(20, 42)
        self.assertEqual(schedule, calibration.schedule(20, 42))
        self.assertEqual(sum(arm == "A" for _, arm, _ in schedule), 20)
        self.assertEqual(sum(order == "AB" for _, _, order in schedule), 20)
        for first, second in zip(schedule[::2], schedule[1::2]):
            self.assertEqual(first[0], second[0])
            self.assertNotEqual(first[1], second[1])

    def test_exact_interval_and_precision_gate(self):
        interval = calibration.median_interval(list(range(20)))
        self.assertEqual(interval["lower_ms"], 5)
        self.assertEqual(interval["upper_ms"], 14)
        self.assertGreaterEqual(interval["coverage"], .95)
        stable = calibration.summarize(self.rows(list(range(-10, 10))), 20)
        self.assertTrue(stable["precision_pass"])
        noisy = calibration.summarize(self.rows([-500, 500] * 10), 20)
        self.assertFalse(noisy["precision_pass"])
        biased = calibration.summarize(self.rows([200] * 20), 20)
        self.assertFalse(biased["precision_pass"])

    def test_missing_failure_and_changed_browser_cannot_pass(self):
        rows = self.rows([0] * 20)
        self.assertFalse(calibration.summarize(rows[:-1], 20)["precision_pass"])
        rows[0]["status"] = "timeout"
        self.assertFalse(calibration.summarize(rows, 20)["precision_pass"])
        rows[0]["status"] = "ok"
        rows[0]["browser"] = {"product": "changed"}
        self.assertFalse(calibration.summarize(rows, 20)["precision_pass"])

    def test_sensitivity_requires_recovering_the_known_delay(self):
        rows = self.rows([500 + i for i in range(-10, 10)])
        self.assertTrue(calibration.summarize(rows, 20, expected_ms=500)["precision_pass"])
        self.assertFalse(calibration.summarize(rows, 20)["precision_pass"])
        self.assertFalse(calibration.summarize(rows, 20, expected_ms=500,
                                              control_baseline_ms=0)["precision_pass"])

    def test_sensitivity_has_a_matched_sham_but_normal_aa_has_no_injection(self):
        self.assertEqual(calibration.control_delays(0), {"A": 0, "B": 0})
        self.assertEqual(calibration.control_delays(500), {"A": 1, "B": 501})
        self.assertEqual(calibration.control_delays(1, 0), {"A": 0, "B": 1})
        for difference, baseline in [(2000, None), (500, -1), (-1, 1), (1.5, 1)]:
            with self.assertRaises(ValueError):
                calibration.control_delays(difference, baseline)

    def test_environment_blocks_builds_missing_access_and_power_changes(self):
        power = {"source": "AC Power", "low_power_mode": 0}
        outputs = {("ps", "-axo", "pid=,comm="): "10 /usr/bin/rustc\n11 /private/personal-app\n",
                   ("pmset", "-g", "batt"): "Now drawing from 'AC Power'",
                   ("pmset", "-g", "custom"): "AC Power:\n lowpowermode 0"}
        receipt = calibration.sample_environment(lambda args: outputs.get(tuple(args)))
        self.assertEqual(receipt["build_processes"], [{"pid": 10, "name": "rustc"}])
        self.assertEqual(calibration.environment_problem(receipt, power),
                         "concurrent build processes detected")
        receipt["build_processes"] = []
        self.assertIsNone(calibration.environment_problem(receipt, power))
        self.assertEqual(calibration.environment_problem(receipt, {**power, "source": "Battery Power"}),
                         "power configuration changed during collection")
        receipt["process_check_available"] = False
        self.assertEqual(calibration.environment_problem(receipt, power),
                         "process admission check unavailable")
        missing = calibration.sample_environment(lambda args: None)
        self.assertFalse(missing["process_check_available"])
        self.assertEqual(missing["power"], {"source": None, "low_power_mode": None})

    def test_environment_failure_cannot_be_counted_as_a_success(self):
        rows = self.rows([0] * 20)
        rows[-1]["status"] = "environment_invalid"
        rows[-1]["runner_status"] = "ok"
        summary = calibration.summarize(rows, 20)
        self.assertFalse(summary["precision_pass"])
        self.assertEqual(summary["successful_runs"], 39)


if __name__ == "__main__":
    unittest.main()
