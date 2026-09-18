import unittest

from perf_environment import capture, power_settings


class EnvironmentTests(unittest.TestCase):
    def test_power_mode_uses_active_source(self):
        custom = "Battery Power:\n lowpowermode 1\nAC Power:\n lowpowermode 0\n"
        self.assertEqual(power_settings("Now drawing from 'AC Power'", custom),
                         {"source": "AC Power", "low_power_mode": 0})
        self.assertEqual(power_settings("Now drawing from 'Battery Power'", custom),
                         {"source": "Battery Power", "low_power_mode": 1})

    def test_unavailable_never_becomes_verified(self):
        record = capture(lambda _: None, "Darwin")
        self.assertFalse(record["host_metadata_complete"])
        self.assertFalse(record["environment_verified"])
        self.assertIsNone(record["power"]["source"])

    def test_no_raw_command_output_is_retained(self):
        record = capture(lambda _: "private/identity\nsecret", "Darwin")
        self.assertNotIn("private", str(record))
        self.assertNotIn("secret", str(record))

    def test_complete_metadata_is_not_profile_verification(self):
        values = {"hw.model": "Mac15,6", "machdep.cpu.brand_string": "Apple M3 Pro",
                  "hw.logicalcpu": "12", "hw.memsize": "38654705664",
                  "batt": "Now drawing from 'AC Power'", "custom": "AC Power:\n lowpowermode 0"}
        record = capture(lambda args: values[args[-1]], "Darwin")
        self.assertTrue(record["host_metadata_complete"])
        self.assertFalse(record["environment_verified"])


if __name__ == "__main__":
    unittest.main()
