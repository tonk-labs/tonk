"""Serve a built Tonk runtime on loopback with no sync backend.

Usage: python3 scripts/serve-local-runtime.py DIST [PORT]
Only static assets and SPA routes are served. Escaped worker API requests fail.
"""

import json
import sys
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit

root = Path(sys.argv[1]).resolve()
if not (root / "index.html").is_file():
    raise SystemExit("DIST must contain a built index.html")


class Handler(SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=str(root), **kwargs)

    def do_GET(self):
        path = urlsplit(self.path).path
        if path in ("/.well-known/tonk", "/__test/direct-build.notation"):
            fixture = Path(__file__).resolve().parent.parent / "examples/direct-build.notation"
            body = json.dumps({}).encode() if path == "/.well-known/tonk" else fixture.read_bytes()
            self.send_response(200)
            self.send_header("Content-Type", "application/json" if path == "/.well-known/tonk" else "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif path.startswith(("/api/", "/ucan/", "/customer/", "/@")):
            self.send_error(503, "Local smoke test has no server API or sync backend")
        else:
            if path == "/" or path.startswith("/space/"):
                self.path = "/index.html"
            super().do_GET()


port = int(sys.argv[2]) if len(sys.argv) > 2 else 4187
print(f"Local-only runtime: http://127.0.0.1:{port}", flush=True)
ThreadingHTTPServer(("127.0.0.1", port), Handler).serve_forever()
