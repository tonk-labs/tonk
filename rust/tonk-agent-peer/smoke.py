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
import os
import subprocess
import sys
import tempfile
import urllib.request

EXPECTED_TOOLS = {
    "tonk_list_spaces", "tonk_join", "tonk_query", "tonk_evaluate", "tonk_guide",
    "tonk_show", "tonk_watch",
}
APP_MIME = "text/html;profile=mcp-app"


class Session:
    def __init__(self, binary, home):
        env = dict(os.environ, TONK_AGENT_PEER_HOME=home)
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
        failed, unjoined = first.tool("tonk_show", {
            "space": "did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK",
            "document": "concept:\n",
        })
        assert failed and "has not joined" in unjoined["content"][0]["text"], unjoined
        assert any(r["uri"] == "tonk://guide/notation" for r in guides), guides
        notation = first.request("resources/read", {"uri": "tonk://guide/notation"})
        assert notation["contents"][0]["text"].strip(), notation

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

        seed = "4" * 44
        failed, refused = second.tool("tonk_join", {
            "link": f"not a link #{seed}"
        })
        text = refused["content"][0]["text"]
        assert failed, refused
        assert seed not in text, refused

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
