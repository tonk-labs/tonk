import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
SCRIPT = REPO_ROOT / "scripts" / "benchmark-wasm-runner.py"


class BenchmarkWasmRunnerCliTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.root = Path(self.temp_dir.name)
        self.bin_dir = self.root / "bin"
        self.bin_dir.mkdir()
        self.archive_dir = self.root / "archive"
        self.archive_dir.mkdir()
        (self.archive_dir / "tests-web-debug.tar.zst").write_bytes(b"same archive")
        (self.archive_dir / "tests-web-release.tar.zst").write_bytes(b"same release archive")
        self.calls = self.root / "calls.jsonl"

        self._write_executable(
            "nix",
            """
            import json
            import os
            from pathlib import Path
            import sys

            with Path(os.environ["FAKE_CALLS"]).open("a") as calls:
                calls.write(json.dumps({"tool": "nix", "args": sys.argv[1:]}) + "\\n")
            print(os.environ["FAKE_ARCHIVE_DIR"])
            """,
        )
        self._write_executable(
            "nix-store",
            """
            import json
            import os
            from pathlib import Path
            import sys

            with Path(os.environ["FAKE_CALLS"]).open("a") as calls:
                calls.write(json.dumps({"tool": "nix-store", "args": sys.argv[1:]}) + "\\n")
            root = Path(sys.argv[sys.argv.index("--add-root") + 1])
            store_path = Path(sys.argv[-1])
            root.symlink_to(store_path, target_is_directory=True)
            print(store_path)
            """,
        )
        self._write_executable(
            "cargo",
            """
            import json
            import os
            from pathlib import Path
            import subprocess
            import sys
            import time

            runner = os.environ.get("CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER")
            call = {"tool": "cargo", "args": sys.argv[1:], "runner": runner}
            with Path(os.environ["FAKE_CALLS"]).open("a") as calls:
                calls.write(json.dumps(call) + "\\n")

            if sys.argv[1:3] == ["nextest", "list"]:
                is_release = any("release" in argument for argument in sys.argv)
                testcases = {
                    "passes": {"kind": "test", "ignored": False},
                    "ignored": {"kind": "test", "ignored": True},
                }
                if is_release:
                    testcases["ignored_release"] = {"kind": "test", "ignored": True}
                print(json.dumps({
                    "test-count": len(testcases),
                    "rust-suites": {
                        "fixture": {
                            "package-name": "fixture",
                            "binary-id": "fixture",
                            "binary-name": "fixture",
                            "kind": "lib",
                            "testcases": testcases,
                        }
                    },
                }))
                raise SystemExit(0)

            is_pool = runner and runner.endswith("wbg-pool")
            if not is_pool and os.environ.get("FAKE_STOCK_HANG") == "1":
                descendant = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"])
                Path(os.environ["FAKE_DESCENDANT_PID"]).write_text(str(descendant.pid))
                time.sleep(60)
            if not is_pool:
                time.sleep(0.2)
            else:
                pool_dir = Path(os.environ["WBG_POOL_DIR"])
                pool_dir.mkdir(parents=True, exist_ok=True)
                daemon_log = pool_dir / "daemon.log"
                if not daemon_log.exists():
                    daemon_log.write_text("wbg-pool daemon listening on http://127.0.0.1:1234\\n")
                if os.environ.get("FAKE_POOL_RELAUNCH") == "1":
                    with daemon_log.open("a") as log:
                        log.write("wbg-pool daemon listening on http://127.0.0.1:5678\\n")
                if os.environ.get("FAKE_REMOVE_POOL_RUNNER") == "1":
                    Path(runner).unlink(missing_ok=True)
            failed = is_pool and os.environ.get("FAKE_POOL_FAIL") == "1"
            print(json.dumps({"type": "suite", "event": "started", "test_count": 1}))
            print(json.dumps({"type": "test", "event": "started", "name": "fixture::fixture$passes"}))
            if failed:
                print(json.dumps({"type": "test", "event": "failed", "name": "fixture::fixture$passes", "exec_time": 0.01}))
                print(json.dumps({"type": "suite", "event": "failed", "passed": 0, "failed": 1, "ignored": 0}))
                print("Summary [0.010s] 1 test run: 0 passed, 1 failed, 1 skipped")
                raise SystemExit(100)
            if os.environ.get("FAKE_MISSING_TERMINAL_EVENTS") != "1":
                print(json.dumps({"type": "test", "event": "ok", "name": "fixture::fixture$passes", "exec_time": 0.01}))
                print(json.dumps({"type": "test", "event": "ignored", "name": "fixture::fixture$ignored"}))
                if any("release" in argument for argument in sys.argv):
                    print(json.dumps({"type": "test", "event": "ignored", "name": "fixture::fixture$ignored_release"}))
            print(json.dumps({"type": "suite", "event": "ok", "passed": 1, "failed": 0, "ignored": 0}))
            skipped = 2 if any("release" in argument for argument in sys.argv) else 1
            print(f"Summary [0.010s] 1 test run: 1 passed, {skipped} skipped")
            """,
        )
        self.pool = self._write_executable(
            "wbg-pool",
            """
            import json
            import os
            from pathlib import Path
            import sys

            with Path(os.environ["FAKE_CALLS"]).open("a") as calls:
                calls.write(json.dumps({"tool": "wbg-pool", "args": sys.argv[1:]}) + "\\n")
            """,
        )
        self.stock = self._write_executable("wasm-bindgen-test-runner", "raise SystemExit(0)")

    def tearDown(self) -> None:
        self.temp_dir.cleanup()

    def _write_executable(self, name: str, body: str) -> Path:
        path = self.bin_dir / name
        path.write_text("#!/usr/bin/env python3\n" + textwrap.dedent(body).lstrip())
        path.chmod(0o755)
        return path

    def _run(
        self,
        *,
        pool_fails: bool = False,
        pool_relaunches: bool = False,
        remove_pool_runner: bool = False,
        missing_terminal_events: bool = False,
        stock_hangs: bool = False,
        profiles: tuple[str, ...] = ("debug",),
    ) -> tuple[subprocess.CompletedProcess[str], Path]:
        output = self.root / "evidence" / "benchmark.json"
        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{self.bin_dir}{os.pathsep}{env['PATH']}",
                "FAKE_ARCHIVE_DIR": str(self.archive_dir),
                "FAKE_CALLS": str(self.calls),
                "WBG_POOL_FALLBACK_RUNNER": str(self.stock),
            }
        )
        if pool_fails:
            env["FAKE_POOL_FAIL"] = "1"
        if pool_relaunches:
            env["FAKE_POOL_RELAUNCH"] = "1"
        if remove_pool_runner:
            env["FAKE_REMOVE_POOL_RUNNER"] = "1"
        if missing_terminal_events:
            env["FAKE_MISSING_TERMINAL_EVENTS"] = "1"
        if stock_hangs:
            env["FAKE_STOCK_HANG"] = "1"
            env["FAKE_DESCENDANT_PID"] = str(self.root / "descendant.pid")
            env["WASM_RUNNER_BENCHMARK_TIMEOUT"] = "1"
        completed = subprocess.run(
            [
                sys.executable,
                str(SCRIPT),
                "--profiles",
                *profiles,
                "--runs",
                "1",
                "--output",
                str(output),
            ],
            cwd=REPO_ROOT,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            check=False,
        )
        return completed, output

    def _calls(self) -> list[dict[str, object]]:
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def test_writes_passing_evidence_from_one_archive_and_one_inventory(self) -> None:
        completed, output = self._run()

        self.assertEqual(completed.returncode, 0, completed.stdout)
        evidence = json.loads(output.read_text())
        profile = evidence["profiles"]["debug"]
        self.assertTrue(evidence["passed"])
        self.assertEqual(profile["archive"]["size_bytes"], len(b"same archive"))
        self.assertEqual(profile["inventory"]["count"], 2)
        self.assertEqual(profile["runners"]["stock"]["runs"][0]["counts"], {"passed": 1, "skipped": 1, "failed": 0})
        self.assertEqual(profile["runners"]["pool"]["runs"][0]["retries"], 0)
        self.assertLessEqual(profile["comparison"]["pooled_to_stock_median_ratio"], 0.5)
        self.assertEqual(evidence["pool_lifecycle"]["daemon_starts"], 1)
        self.assertEqual(len(evidence["gc_roots"]), 3)
        for root in evidence["gc_roots"]:
            self.assertFalse(Path(root["path"]).exists())
        self.assertEqual(
            Path(evidence["pool_lifecycle"]["state_dir"]).parent,
            output.parent.resolve(),
        )

        calls = self._calls()
        self.assertEqual(sum(call["tool"] == "nix" for call in calls), 1)
        self.assertEqual(sum(call["tool"] == "nix-store" for call in calls), 3)
        cargo_calls = [call for call in calls if call["tool"] == "cargo"]
        self.assertEqual(sum(call["args"][:2] == ["nextest", "list"] for call in cargo_calls), 1)
        run_calls = [call for call in cargo_calls if call["args"][:2] == ["nextest", "run"]]
        self.assertEqual(len(run_calls), 2)
        for call in run_calls:
            self.assertIn("--test-threads", call["args"])
            self.assertIn("4", call["args"])
            self.assertIn("--retries", call["args"])
            self.assertIn("0", call["args"])
            self.assertIn(str(self.archive_dir / "tests-web-debug.tar.zst"), call["args"])
        self.assertEqual(
            [call["args"] for call in calls if call["tool"] == "wbg-pool"],
            [["daemon", "--stop"], ["daemon", "--stop"]],
        )
        for runner_name in ("stock", "pool"):
            log_path = Path(profile["runners"][runner_name]["runs"][0]["log"])
            self.assertTrue(log_path.is_file())
            self.assertIn("Summary", log_path.read_text())

    def test_returns_nonzero_and_records_reason_when_outcomes_differ(self) -> None:
        completed, output = self._run(pool_fails=True)

        self.assertEqual(completed.returncode, 1, completed.stdout)
        evidence = json.loads(output.read_text())
        self.assertFalse(evidence["passed"])
        comparison = evidence["profiles"]["debug"]["comparison"]
        self.assertFalse(comparison["passed"])
        self.assertTrue(any("exit status" in reason for reason in comparison["failures"]))
        self.assertTrue(any("counts" in reason for reason in comparison["failures"]))

    def test_returns_nonzero_when_the_pool_relaunches_during_a_run(self) -> None:
        completed, output = self._run(pool_relaunches=True)

        self.assertEqual(completed.returncode, 1, completed.stdout)
        evidence = json.loads(output.read_text())
        self.assertFalse(evidence["passed"])
        self.assertEqual(evidence["pool_lifecycle"]["daemon_starts"], 2)
        self.assertTrue(
            any("started 2 daemons" in failure for failure in evidence["failures"]),
            evidence["failures"],
        )

    def test_returns_nonzero_when_terminal_test_events_are_incomplete(self) -> None:
        completed, output = self._run(missing_terminal_events=True)

        self.assertEqual(completed.returncode, 1, completed.stdout)
        evidence = json.loads(output.read_text())
        comparison = evidence["profiles"]["debug"]["comparison"]
        self.assertTrue(
            any("terminal test outcomes" in failure for failure in comparison["failures"]),
            comparison["failures"],
        )

    def test_timeout_stops_descendant_process_group(self) -> None:
        completed, output = self._run(stock_hangs=True)

        self.assertEqual(completed.returncode, 1, completed.stdout)
        evidence = json.loads(output.read_text())
        stock_run = evidence["profiles"]["debug"]["runners"]["stock"]["runs"][0]
        self.assertTrue(stock_run["timed_out"])
        descendant_pid = int((self.root / "descendant.pid").read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(descendant_pid, 0)

    def test_records_cleanup_failure_when_pool_runner_disappears(self) -> None:
        completed, output = self._run(remove_pool_runner=True)

        self.assertEqual(completed.returncode, 1, completed.stdout)
        self.assertNotIn("Traceback", completed.stdout)
        evidence = json.loads(output.read_text())
        self.assertFalse(evidence["passed"])
        self.assertTrue(
            any("stopping wbg-pool after benchmark failed" in failure for failure in evidence["failures"]),
            evidence["failures"],
        )

    def test_keeps_all_pooled_profiles_contiguous(self) -> None:
        completed, output = self._run(profiles=("debug", "release"))

        self.assertEqual(completed.returncode, 0, completed.stdout)
        evidence = json.loads(output.read_text())
        for profile in evidence["profiles"].values():
            for runner in profile["runners"].values():
                self.assertTrue(runner["runs"][0]["inventory_matches"])
        run_calls = [
            call
            for call in self._calls()
            if call["tool"] == "cargo" and call["args"][:2] == ["nextest", "run"]
        ]
        self.assertEqual(
            [Path(call["args"][call["args"].index("--archive-file") + 1]).name for call in run_calls],
            [
                "tests-web-debug.tar.zst",
                "tests-web-release.tar.zst",
                "tests-web-debug.tar.zst",
                "tests-web-release.tar.zst",
            ],
        )
        self.assertEqual(
            [str(call["runner"]).endswith("wbg-pool") for call in run_calls],
            [False, False, True, True],
        )


if __name__ == "__main__":
    unittest.main()
