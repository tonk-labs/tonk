"""Run the built native app through the shared MCP workflow and a process restart.

Serve a local runtime on port 4187 first. Every run uses new application and
WebKit data; no model login, real account or cloud backend is used.
"""
import json
import plistlib
import shutil
import subprocess
import sys
import tempfile
import time
import uuid
from pathlib import Path

desktop = Path(__file__).resolve().parent.parent
root = desktop.parent
app = desktop / ".build/Tonk.app/Contents/MacOS/Tonk"
if not app.is_file():
    raise SystemExit("Build desktop/.build/Tonk.app first")
(desktop / "artifacts").mkdir(exist_ok=True)
directory = Path(tempfile.mkdtemp(prefix="agent-workflow-", dir=desktop / "artifacts"))
profile = str(uuid.uuid4())
test_app = directory / "Tonk Workflow.app"
shutil.copytree(app.parents[2], test_app)
plist = test_app / "Contents/Info.plist"
metadata = plistlib.loads(plist.read_bytes())
metadata["CFBundleIdentifier"] = "xyz.tonk.workflow." + profile
plist.write_bytes(plistlib.dumps(metadata))
subprocess.run(["codesign", "--force", "--sign", "-", str(test_app)], check=True)
print(directory, flush=True)
for phase in ("create", "reopen"):
    report = directory / (phase + ".json")
    command = ["open", "-n", "-W", str(test_app), "--args", "--local-runtime", "--web-data-id", profile,
               "--data-dir", str(directory / "data"), "--agent-workflow-test",
               "--workflow-fixture", str(root / "mcp/fixtures/authoring/tasks.notation"),
               "--report", str(report)]
    if phase == "reopen":
        command.append("--workflow-reopen")
    with (directory / (phase + ".log")).open("w") as output:
        child = subprocess.Popen(command, stdout=output, stderr=subprocess.STDOUT)
        time.sleep(1)
        subprocess.run(["open", str(test_app)], check=True)
        try:
            code = child.wait(timeout=180)
        except subprocess.TimeoutExpired:
            child.terminate()
            child.wait(timeout=10)
            raise SystemExit("Native workflow timed out; inspect " + str(directory))
    result = json.loads(report.read_text()) if report.exists() else {"error": "No report", "exit": code}
    print(json.dumps({"phase": phase, "report": str(report), "passed": result.get("passed", False),
                      "checks": result.get("checks", []), "error": result.get("error")}, indent=2), flush=True)
    if not result.get("passed"):
        sys.exit(1)
print("PASS: native MCP authoring, rendering and process-restart persistence", flush=True)
