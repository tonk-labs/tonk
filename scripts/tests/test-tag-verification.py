import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("gate", Path(__file__).parents[1] / "wait-tag-verification.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


def run(**updates):
    return dict(id=10, run_attempt=1, head_sha="abc", head_branch="v1.0.0",
                event="push", path=gate.WORKFLOW, status="completed", conclusion="success") | updates


class GateTests(unittest.TestCase):
    def check(self, runs, detail=None, **options):
        ticks = [0]
        def sleep(seconds):
            ticks[0] += seconds
        def get(path):
            if "?" in path:
                return {"workflow_runs": runs}
            return detail or gate.latest(runs, "abc", "v1.0.0")
        return gate.wait(get, "abc", "refs/tags/v1.0.0", timeout=3, interval=1,
                         clock=lambda: ticks[0], sleep=sleep, **options)

    def test_exact_identity(self):
        for field, wrong in [("head_sha", "def"), ("head_branch", "main"),
                             ("head_branch", "v1.0.1"), ("event", "workflow_dispatch"),
                             ("path", ".github/workflows/release.yml")]:
            with self.subTest(field=field, wrong=wrong), self.assertRaises(TimeoutError):
                self.check([run(**{field: wrong})])
        self.assertEqual(self.check([run()])["id"], 10)

    def test_latest_run_and_attempt(self):
        with self.assertRaisesRegex(RuntimeError, "failure"):
            self.check([run(), run(id=11, conclusion="failure")])
        with self.assertRaisesRegex(RuntimeError, "failure"):
            self.check([run(), run(run_attempt=2, conclusion="failure")])
        with self.assertRaisesRegex(RuntimeError, "failure"):
            self.check([run()], detail=run(run_attempt=2, conclusion="failure"))
        with self.assertRaisesRegex(RuntimeError, "older attempt"):
            self.check([run(run_attempt=2)], detail=run())

    def test_unsuccessful_conclusions(self):
        for conclusion in ["failure", "cancelled", "timed_out", "skipped", "neutral", None]:
            with self.subTest(conclusion=conclusion), self.assertRaises(RuntimeError):
                self.check([run(conclusion=conclusion)])

    def test_pending_and_missing_timeout(self):
        for runs in [[], [run(status="queued")], [run(status="in_progress")]]:
            with self.assertRaises(TimeoutError):
                self.check(runs)

    def test_poll_then_success(self):
        calls = []
        def get(path):
            calls.append(path)
            if "?" in path:
                return {"workflow_runs": [] if len(calls) == 1 else [run()]}
            return run()
        ticks = [0]
        def sleep(seconds):
            ticks[0] += seconds
        self.assertEqual(gate.wait(get, "abc", "refs/tags/v1.0.0", timeout=3,
                         clock=lambda: ticks[0], sleep=sleep, interval=1)["id"], 10)

    def test_api_failure(self):
        def get(_):
            raise OSError("API unavailable")
        with self.assertRaises(OSError):
            gate.wait(get, "abc", "refs/tags/v1.0.0")

    def test_ref_and_refreshed_identity(self):
        with self.assertRaises(ValueError):
            gate.wait(lambda _: None, "abc", "refs/heads/main")
        with self.assertRaisesRegex(RuntimeError, "identity"):
            self.check([run()], detail=run(head_branch="main"))

    def test_pagination(self):
        def get(path):
            return {"workflow_runs": [run(head_branch="main")] * 100 if path.endswith("&page=1") else [run()]}
        self.assertEqual(len(gate.list_runs(get, "abc", "v1.0.0")), 101)
        with self.assertRaisesRegex(RuntimeError, "pagination"):
            gate.list_runs(lambda _: {"workflow_runs": [run()] * 100}, "abc", "v1.0.0")


if __name__ == "__main__":
    unittest.main()
