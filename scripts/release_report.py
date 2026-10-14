#!/usr/bin/env python3
"""Render release evidence without executing or embedding uploaded content."""
import argparse
import html
import json
from pathlib import Path
import re

from release_gate import evaluate, load


def markdown(report):
    def text(value):
        value = re.sub(r"([\\`*_{}\[\]()#+!])", r"\\\1", str(value).replace("\n", " "))
        return html.escape(value).replace("|", "&#124;")
    context = report["candidate"].get("context", {})
    lines = ["# Release readiness", "", "**" + ("Ready for approval" if report["ready"] else "Promotion blocked") + "**",
             "", f"Candidate: `{text(context.get('version', '?'))}` at `{text(context.get('source_sha', '?'))}`",
             f"Run: {text(context.get('run_id', '?'))}, attempt {text(context.get('run_attempt', '?'))}",
             "", "Required browser and CLI release flows. This is not a claim of complete product coverage.",
             "", "Observation phase: existing release publishing is not gated by this report.",
             "", "| Flow | Automated candidate | Human verification |", "| --- | --- | --- |"]
    lines += [f"| {text(row['title'])} | {text(row['automated'])} | {text(row['manual'])} |"
              for row in report["flows"]]
    lines += ["", "## Required automated checks", "", "| Check | Status |", "| --- | --- |"]
    lines += [f"| {text(row['id'])} | {text(row['status'])} |" for row in report.get("checks", [])]
    lines += ["", "chatgpt covers automated authorization, session and packaging contracts; actual ChatGPT linking and tool use are not verified here."]
    if report["blockers"]:
        lines += ["", "## Blockers", ""] + ["- " + text(reason) for reason in report["blockers"]]
    return "\n".join(lines) + "\n"


def render_html(report):
    esc = lambda value: html.escape(str(value), quote=True)
    context = report["candidate"].get("context", {})
    rows = []
    for row in report["flows"]:
        details = "<br>".join(esc(test) for test in row["tests"])
        rows.append(f"<tr><th scope='row'>{esc(row['title'])}<details><summary>Evidence mapping</summary>"
                    f"<p>{esc(', '.join(row['journeys']))}</p><code>{details}</code></details></th>"
                    f"<td>{esc(row['automated'])}</td><td>{esc(row['manual'])}</td></tr>")
    checks = "".join(f"<tr><th scope='row'>{esc(row['id'])}</th><td>{esc(row['status'])}</td></tr>"
                     for row in report.get("checks", []))
    blockers = "".join(f"<li>{esc(value)}</li>" for value in report["blockers"])
    status = "Ready for approval" if report["ready"] else "Promotion blocked"
    fixture = "<p><strong>Synthetic fixture preview; these are not product test results.</strong></p>" if report.get("synthetic") else ""
    return f"""<!doctype html>
<html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'">
<title>Release readiness · {esc(context.get('version', '?'))}</title>
<style>
:root{{color-scheme:light dark;--page:#e8e6e4;--ink:#38182a;--panel:#f7f6f5;--line:#b6a9b0}}
@media(prefers-color-scheme:dark){{:root{{--page:#161313;--ink:#e2dfdd;--panel:#261f20;--line:#62555d}}}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--page);color:var(--ink);font:16px/1.5 system-ui,sans-serif}}
main{{max-width:1100px;margin:auto;padding:32px 20px}}h1{{font-size:30px;margin:0 0 20px}}
.status{{border:2px solid var(--ink);padding:16px;font-size:20px;font-weight:600}}
code{{font-size:13px;overflow-wrap:anywhere}}.table{{overflow-x:auto}}table{{width:100%;border-collapse:collapse;background:var(--panel)}}
th,td{{text-align:left;vertical-align:top;padding:16px;border-bottom:1px solid var(--line)}}
thead th{{font-size:14px}}tbody th{{font-weight:500}}summary{{cursor:pointer;font-size:14px}}details{{margin-top:8px;font-weight:400}}
li{{margin-bottom:8px;overflow-wrap:anywhere}}@media(max-width:600px){{main{{padding:20px 12px}}th,td{{padding:10px}}}}
</style><main><h1>Release readiness</h1>{fixture}<p class="status">{status}</p>
<p><strong>Observation phase:</strong> existing release publishing is not gated by this report.</p>
<p>Version <strong>{esc(context.get('version', '?'))}</strong> · {esc(context.get('channel', '?'))}</p>
<p>Source <code>{esc(context.get('source_sha', '?'))}</code><br>
Run {esc(context.get('run_id', '?'))}, attempt {esc(context.get('run_attempt', '?'))}</p>
<p>Required browser and CLI release flows. Source fixtures and historical screenshots do not count as verification.</p>
<div class="table"><table><thead><tr><th scope="col">Flow</th><th scope="col">Automated candidate</th>
<th scope="col">Human verification</th></tr></thead><tbody>{''.join(rows)}</tbody></table></div>
<h2>Required automated checks</h2>
<table><thead><tr><th scope="col">Check</th><th scope="col">Status</th></tr></thead><tbody>{checks}</tbody></table>
<p>chatgpt covers automated authorization, session and packaging contracts; actual ChatGPT linking and tool use are not verified here.</p>
<h2>Blockers ({len(report['blockers'])})</h2><ul>{blockers}</ul>
<p>Generated {esc(report['evaluated_at'])}. Approval and publication happen through GitHub Actions.
This report cannot approve a release.</p></main></html>\n"""


def write(report, output):
    output = Path(output)
    output.mkdir(parents=True, exist_ok=True)
    (output / "readiness.json").write_text(json.dumps(report, indent=2) + "\n")
    (output / "readiness.md").write_text(markdown(report))
    (output / "index.html").write_text(render_html(report))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--evidence", required=True)
    parser.add_argument("--policy", default="release/flows.json")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    result = evaluate(load(args.candidate), load(args.policy), load(args.evidence))
    write(result, args.output)
    raise SystemExit(0 if result["ready"] else 1)
