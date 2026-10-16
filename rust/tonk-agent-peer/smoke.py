#!/usr/bin/env python3
"""Drive a built tonk-agent-peer over MCP stdio in a throwaway home.

    python3 rust/tonk-agent-peer/smoke.py path/to/tonk-agent-peer

Checks the handshake, the tool and guide catalogs, that a fresh peer has an
identity and no spaces, that a second process sharing the directory forwards
to the first and sees the same peer, and that a link for another service is
refused. Needs no network.
"""
import json
import os
import subprocess
import sys
import tempfile

EXPECTED_TOOLS = {"tonk_list_spaces", "tonk_join", "tonk_query", "tonk_evaluate", "tonk_guide"}


class Session:
    def __init__(self, binary, home):
        env = dict(os.environ, TONK_AGENT_PEER_HOME=home, TONK_AGENT_PEER_REMOTE="https://tonk.network/ucan/")
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
        tools = {tool["name"] for tool in first.request("tools/list")["tools"]}
        assert tools == EXPECTED_TOOLS, tools
        guides = first.request("resources/list")["resources"]
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
            "link": f"https://evil.example/space/did:key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK#{seed}"
        })
        text = refused["content"][0]["text"]
        assert failed and "different Tonk service" in text, refused
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
