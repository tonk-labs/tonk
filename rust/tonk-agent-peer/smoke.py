#!/usr/bin/env python3
"""Drive a built tonk-agent-peer over MCP stdio in a throwaway home.

    python3 rust/tonk-agent-peer/smoke.py path/to/tonk-agent-peer

Checks the handshake, the tool and guide catalogs, the live-view app (its
tool metadata, ui:// resource and loopback probe), that a fresh peer has an
identity and no spaces, that a second process sharing the directory forwards
to the first and sees the same peer, and that a malformed link is refused
without echoing its secret. Needs no network.
"""
import json
import re
import shutil
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request

EXPECTED_TOOLS = {
    "tonk_list_spaces", "tonk_join", "tonk_query", "tonk_evaluate", "tonk_guide",
    "tonk_show", "tonk_watch", "tonk_display", "tonk_view_log",
}
APP_MIME = "text/html;profile=mcp-app"
# A stand-in guest runtime directory; the real pack is built from tonk-ui.
RUNTIME = tempfile.mkdtemp()
with open(os.path.join(RUNTIME, "space-runtime.json"), "w") as stand_in:
    stand_in.write('{"stand_in":true}')


class Session:
    def __init__(self, binary, home):
        env = dict(os.environ, TONK_AGENT_PEER_HOME=home, TONK_AGENT_PEER_RUNTIME=RUNTIME)
        self.process = subprocess.Popen(
            [binary], stdin=subprocess.PIPE, stdout=subprocess.PIPE, env=env, text=True
        )
        self.next_id = 0
        info = self.request("initialize", {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "tonk-agent-peer-smoke", "version": "0"},
        })
        assert info["serverInfo"]["name"] == "tonk", info
        assert "tools" in info["capabilities"], info
        self.send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()

    def request(self, method, params=None):
        self.next_id += 1
        self.send({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params or {}})
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise SystemExit(f"server closed stdout during {method}")
            message = json.loads(line)
            if message.get("id") == self.next_id:
                if "error" in message:
                    raise SystemExit(f"{method} failed: {message['error']}")
                return message["result"]

    def tool(self, name, arguments=None):
        result = self.request("tools/call", {"name": name, "arguments": arguments or {}})
        return result.get("isError", False), result

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=30)


def main(binary):
    with tempfile.TemporaryDirectory() as scratch:
        home = os.path.join(scratch, "peer")
        first = Session(binary, home)
        listed_tools = {tool["name"]: tool for tool in first.request("tools/list")["tools"]}
        assert set(listed_tools) == EXPECTED_TOOLS, set(listed_tools)
        view_uri = listed_tools["tonk_show"]["_meta"]["ui"]["resourceUri"]
        assert view_uri.startswith("ui://tonk/"), listed_tools["tonk_show"]
        assert listed_tools["tonk_watch"]["_meta"]["ui"]["visibility"] == ["app"]
        guides = first.request("resources/list")["resources"]
        assert any(r["uri"] == view_uri and r.get("mimeType") == APP_MIME for r in guides), guides
        view = first.request("resources/read", {"uri": view_uri})["contents"][0]
        assert view["mimeType"] == APP_MIME and "<html" in view["text"], view.keys()
        assert "__TONK_PROBE_ORIGIN__" not in view["text"]
        probe_origin = view["_meta"]["ui"]["csp"]["connectDomains"][0]
        assert probe_origin.startswith("http://127.0.0.1:"), view["_meta"]
        assert json.dumps(probe_origin) in view["text"]
        with urllib.request.urlopen(probe_origin + "/probe", timeout=5) as response:
            assert response.headers["Access-Control-Allow-Origin"] == "*"
            assert response.headers["Access-Control-Allow-Private-Network"] == "true"
            assert json.load(response)["probe"] is True
        # The space app reaches the owner's worker API on loopback.
        space_uri = listed_tools["tonk_display"]["_meta"]["ui"]["resourceUri"]
        assert any(r["uri"] == space_uri and r.get("mimeType") == APP_MIME for r in guides), guides
        space = first.request("resources/read", {"uri": space_uri})["contents"][0]
        peer_origin = space["_meta"]["ui"]["csp"]["connectDomains"][0]
        assert peer_origin.startswith("http://127.0.0.1:"), space["_meta"]
        assert space["_meta"]["ui"]["csp"]["frameDomains"] == [peer_origin], space["_meta"]
        assert json.dumps(peer_origin) in space["text"] and "__TONK_PEER_ORIGIN__" not in space["text"]
        # The view carries the peer's token; nothing else may use the API.
        match = re.search(r'token = "([0-9a-f]{64})"', space["text"])
        assert match and "__TONK_PEER_TOKEN__" not in space["text"], space["text"][:200]
        token = {"x-tonk-peer-token": match.group(1)}

        def peer_get(path, timeout=5):
            return urllib.request.urlopen(
                urllib.request.Request(peer_origin + path, headers=token), timeout=timeout)

        for path, headers in [("/api/profile", {}), ("/api/profile", {"x-tonk-peer-token": "0" * 64})]:
            try:
                urllib.request.urlopen(
                    urllib.request.Request(peer_origin + path, headers=headers), timeout=5)
                raise AssertionError("the peer API must refuse requests without its token")
            except urllib.error.HTTPError as refused:
                assert refused.code == 401, refused.code
                assert refused.headers["Access-Control-Allow-Origin"] == "*"
        with peer_get("/space-runtime.json") as response:
            assert response.headers["Access-Control-Allow-Origin"] == "*"
            assert json.load(response)["stand_in"] is True
        preflight = urllib.request.Request(
            peer_origin + "/api/profile", method="OPTIONS",
            headers={"Origin": "null", "Access-Control-Request-Method": "POST",
                     "Access-Control-Request-Headers": "content-type,x-tonk-client-id"},
        )
        with urllib.request.urlopen(preflight, timeout=5) as response:
            assert response.status == 204, response.status
            assert response.headers["Access-Control-Allow-Private-Network"] == "true"
            assert "x-tonk-client-id" in response.headers["Access-Control-Allow-Headers"]
        with peer_get("/api/profile", timeout=30) as response:
            assert response.headers["Access-Control-Allow-Origin"] == "*"
            assert response.status == 200
        try:
            peer_get("/worker-guest.html")
            raise AssertionError("missing guest document must 404")
        except urllib.error.HTTPError as missing:
            assert missing.code == 404 and "not installed" in missing.read().decode()
        failed, log = first.tool("tonk_view_log")
        assert not failed and any("/space-runtime.json -> 200" in line
                                  for line in log["structuredContent"]["log"]), log
        with peer_get("/display-guest.html") as response:
            assert "tonk-display-mount" in response.read().decode()
        failed, closed = first.tool("tonk_display", {
            "space": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
            "items": [{"entity": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK", "model": "tonk/repository"}],
        })
        assert failed and "has not joined" in closed["content"][0]["text"], closed
        failed, unjoined = first.tool("tonk_show", {
            "space": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
            "document": "concept:\n",
        })
        assert failed and "has not joined" in unjoined["content"][0]["text"], unjoined
        assert any(r["uri"] == "tonk://guide/notation" for r in guides), guides
        notation = first.request("resources/read", {"uri": "tonk://guide/notation"})
        assert notation["contents"][0]["text"].strip(), notation
        # MCP requires structuredContent to be an object; Claude Desktop
        # rejects the whole result when a guide comes back as a string.
        failed, guide = first.tool("tonk_guide", {"topic": "notation"})
        assert not failed and guide["content"][0]["text"].strip(), guide
        assert isinstance(guide.get("structuredContent", {}), dict), guide

        failed, listed = first.tool("tonk_list_spaces")
        assert not failed, listed
        peer = listed["structuredContent"]["peer"]
        assert peer.startswith("did:key:"), listed
        assert listed["structuredContent"]["spaces"] == [], listed

        # A second session on the same directory forwards to the owner.
        second = Session(binary, home)
        failed, again = second.tool("tonk_list_spaces")
        assert not failed, again
        assert again["structuredContent"]["peer"] == peer, again
        # Both name the same owning process, which answers for every session.
        owner = listed["structuredContent"]["owner"]
        assert owner["pid"] and owner["version"] and owner["binary"], listed
        assert again["structuredContent"]["owner"] == owner, again

        seed = "4" * 44
        failed, refused = second.tool("tonk_join", {
            "link": f"not a link #{seed}"
        })
        text = refused["content"][0]["text"]
        assert failed, refused
        assert seed not in text, refused

        # A newer build (a later-installed binary) takes over from an older
        # owner; the older session keeps working by forwarding to it.
        newer = os.path.join(scratch, "newer", "tonk-agent-peer")
        os.makedirs(os.path.dirname(newer))
        shutil.copy2(binary, newer)
        later = os.stat(binary).st_mtime + 60
        os.utime(newer, (later, later))
        third = Session(newer, home)
        failed, taken = third.tool("tonk_list_spaces")
        assert not failed and taken["structuredContent"]["peer"] == peer, taken
        assert taken["structuredContent"]["owner"]["pid"] == third.process.pid, taken
        failed, forwarded = first.tool("tonk_list_spaces")
        assert not failed, forwarded
        assert forwarded["structuredContent"]["owner"]["pid"] == third.process.pid, forwarded
        third.close()

        # When the owner's session ends, the other takes over the same peer.
        first.close()
        failed, after = second.tool("tonk_list_spaces")
        assert not failed and after["structuredContent"]["peer"] == peer, after
        second.close()
    print("tonk-agent-peer smoke passed")


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    main(sys.argv[1])
