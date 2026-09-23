"""Generate a local, inert desktop deep-link fixture; never opens or sends it."""

import argparse
from html import escape, unescape
from pathlib import Path
import re
from urllib.parse import quote, parse_qs, urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[2]
    source = (root / "rust/tonk-core/assets/library/onboarding-agent.yaml").read_text()
    match = re.search(r'error-label="could not copy" value="(.*?)">', source, re.S)
    if match is None:
        raise SystemExit("Cannot find onboarding prompt; reconcile fixture with current markup")
    prompt = unescape(match.group(1))
    alphabet = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz"
    link = "https://example.invalid/connect?agent=" + (alphabet * 145)[:8192]
    link += "#tonk-agent-v2=" + "1" * 44
    prompt = (
        "TONK DEEP LINK SPIKE — DUMMY DATA — DO NOT SEND\n"
        + prompt.replace("{name}", 'Spike & café "quoted" space').replace("{link}", link)
        + "\nEND TONK SPIKE — encoding: # & + % ? café 日本語"
    )
    page = (
        '<meta charset="utf-8"><title>Tonk deep link spike</title>'
        "<h1>Tonk deep link spike</h1><p>Dummy data. Do not send the prompt.</p>"
    )
    for name, base, parameter in [
        ("Claude", "claude://code/new", "q"),
        ("Codex", "codex://new", "prompt"),
    ]:
        url = base + "?" + parameter + "=" + quote(prompt, safe="")
        assert parse_qs(urlsplit(url).query)[parameter] == [prompt]
        page += f'<p><a href="{escape(url, quote=True)}">Open {name} spike</a></p>'
        print(f"{name}: {len(prompt)} prompt characters; {len(url)} URL characters")
    page += '<textarea aria-label="Expected prompt" rows="5" cols="80">'
    page += escape(prompt) + "</textarea>"
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "prompt.txt").write_text(prompt)
    (args.output / "index.html").write_text(page)


if __name__ == "__main__":
    main()
