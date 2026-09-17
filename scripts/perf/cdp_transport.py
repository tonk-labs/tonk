#!/usr/bin/env python3
"""Bounded, dependency-free direct-CDP transport collector for synthetic probes."""

import argparse
import base64
import hashlib
import http.client
import ipaddress
import json
import math
import os
from pathlib import Path
import socket
import struct
import sys
import time
from urllib.parse import urlsplit


SCHEMA = 1
GUID = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11"
MAX_HTTP_BODY = 1 << 20
MAX_HEADERS = 1 << 16
MAX_FRAME = 1 << 24
MAX_MESSAGES = 50_000
MAX_CATEGORY_REQUESTS = 32
MAX_TARGETS = 128
SETUP_SETTLE_SECONDS = 0.1
STOP_QUIET_SECONDS = 1.0
STOP_DRAIN_SECONDS = 2.0
PATHS = {
    "/__tonk_transport_probe__/root.txt": "root",
    "/__tonk_transport_probe__/dedicated.txt": "dedicated_worker",
    "/__tonk_transport_probe__/service-worker.txt": "service_worker",
    "/__tonk_transport_probe__/opaque.js": "opaque_iframe",
    "/__tonk_transport_probe__/opaque-late.js": "opaque_iframe_late",
    "/__tonk_transport_probe__/oopif.txt": "cross_origin_iframe",
}
CATEGORIES = tuple(PATHS.values())
TARGET_TYPES = {"page", "iframe", "worker", "service_worker", "shared_worker"}
RECEIPT_TARGET_TYPES = TARGET_TYPES | {"browser"}
SETUP_METHODS = {
    "Network.enable",
    "Runtime.runIfWaitingForDebugger",
    "Target.autoAttachRelated",
    "Target.getTargets",
    "Target.setAutoAttach",
}
ATTACH_MODES = {"recursive", "auto-attach-related"}


class TransportError(RuntimeError):
    """A bounded local transport or CDP protocol failure."""


class Closed(TransportError):
    """The browser completed the WebSocket close handshake."""


def require(condition, message):
    if not condition:
        raise TransportError(message)


def parse_debugger_address(value):
    require(isinstance(value, str) and value and "/" not in value, "invalid debugger address")
    parsed = urlsplit("//" + value)
    require(parsed.username is None and parsed.password is None, "debugger address cannot contain credentials")
    require(parsed.hostname is not None and parsed.port is not None, "debugger address requires host and port")
    host = parsed.hostname
    if host != "localhost":
        try:
            require(ipaddress.ip_address(host).is_loopback, "debugger address must be loopback")
        except ValueError as error:
            raise TransportError("debugger address must be loopback") from error
    return host, parsed.port


def discover_websocket(address, timeout=5.0):
    host, port = parse_debugger_address(address)
    connection = http.client.HTTPConnection(host, port, timeout=timeout)
    try:
        connection.request("GET", "/json/version", headers={"Accept": "application/json"})
        response = connection.getresponse()
        require(response.status == 200, "Chrome debugger version endpoint failed")
        payload = response.read(MAX_HTTP_BODY + 1)
        require(len(payload) <= MAX_HTTP_BODY, "Chrome debugger version response exceeded limit")
    finally:
        connection.close()
    try:
        document = json.loads(payload)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise TransportError("Chrome debugger version response was invalid JSON") from error
    require(isinstance(document, dict), "Chrome debugger version response must be an object")
    endpoint = document.get("webSocketDebuggerUrl")
    require(isinstance(endpoint, str), "Chrome debugger browser WebSocket is unavailable")
    parsed = urlsplit(endpoint)
    require(parsed.scheme == "ws" and parsed.hostname is not None, "Chrome debugger returned an invalid WebSocket")
    returned_host = parsed.hostname
    if returned_host != "localhost":
        try:
            require(ipaddress.ip_address(returned_host).is_loopback, "Chrome debugger WebSocket must be loopback")
        except ValueError as error:
            raise TransportError("Chrome debugger WebSocket must be loopback") from error
    require(parsed.port == port, "Chrome debugger WebSocket changed port")
    require(parsed.path.startswith("/devtools/browser/"), "Chrome debugger returned a non-browser WebSocket")
    return endpoint


class WebSocket:
    """Small RFC 6455 client sufficient for the local browser CDP endpoint."""

    def __init__(self, stream, buffered=b""):
        self.stream = stream
        self.buffered = bytearray(buffered)
        self.fragment_opcode = None
        self.fragments = bytearray()
        self.close_sent = False

    @classmethod
    def connect(cls, endpoint, timeout=5.0):
        parsed = urlsplit(endpoint)
        require(parsed.scheme == "ws" and parsed.hostname and parsed.port, "unsupported WebSocket endpoint")
        host = parsed.hostname
        stream = socket.create_connection((host, parsed.port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        path = parsed.path or "/"
        if parsed.query:
            path += "?" + parsed.query
        host_header = f"[{host}]:{parsed.port}" if ":" in host else f"{host}:{parsed.port}"
        request = (
            f"GET {path} HTTP/1.1\r\nHost: {host_header}\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        ).encode("ascii")
        try:
            stream.sendall(request)
            headers = bytearray()
            while b"\r\n\r\n" not in headers:
                chunk = stream.recv(4096)
                require(chunk, "WebSocket handshake closed early")
                headers.extend(chunk)
                require(len(headers) <= MAX_HEADERS, "WebSocket handshake headers exceeded limit")
            raw_headers, buffered = bytes(headers).split(b"\r\n\r\n", 1)
            cls._verify_handshake(raw_headers, key)
            return cls(stream, buffered)
        except Exception:
            stream.close()
            raise

    @staticmethod
    def _verify_handshake(raw_headers, key):
        try:
            lines = raw_headers.decode("ascii").split("\r\n")
        except UnicodeDecodeError as error:
            raise TransportError("WebSocket handshake headers were not ASCII") from error
        require(lines[0].startswith("HTTP/1.1 101 "), "WebSocket upgrade was rejected")
        headers = {}
        for line in lines[1:]:
            require(":" in line, "malformed WebSocket handshake header")
            name, value = line.split(":", 1)
            headers.setdefault(name.strip().lower(), []).append(value.strip())
        tokens = lambda name: {
            token.strip().lower()
            for value in headers.get(name, [])
            for token in value.split(",")
        }
        require("websocket" in tokens("upgrade"), "WebSocket upgrade header missing")
        require("upgrade" in tokens("connection"), "WebSocket connection header missing")
        expected = base64.b64encode(hashlib.sha1((key + GUID).encode("ascii")).digest()).decode("ascii")
        require(headers.get("sec-websocket-accept") == [expected], "WebSocket accept key mismatch")
        require("sec-websocket-extensions" not in headers, "unrequested WebSocket extension")
        require("sec-websocket-protocol" not in headers, "unrequested WebSocket subprotocol")

    def _fill(self, length, deadline):
        while len(self.buffered) < length:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError
            self.stream.settimeout(remaining)
            chunk = self.stream.recv(min(65536, length - len(self.buffered)))
            if not chunk:
                raise Closed("WebSocket closed without a close frame")
            self.buffered.extend(chunk)

    def _read_frame(self, deadline):
        # Leave every byte buffered until a complete frame is available. A
        # socket timeout can then be retried without losing a partial header,
        # extended length, or payload.
        self._fill(2, deadline)
        first, second = self.buffered[:2]
        header_length = 2
        length = second & 0x7F
        if length == 126:
            self._fill(4, deadline)
            length = struct.unpack("!H", self.buffered[2:4])[0]
            header_length = 4
        elif length == 127:
            self._fill(10, deadline)
            raw = self.buffered[2:10]
            require(not (raw[0] & 0x80), "invalid 64-bit WebSocket length")
            length = struct.unpack("!Q", raw)[0]
            header_length = 10
        require(length <= MAX_FRAME, "WebSocket frame exceeded limit")
        self._fill(header_length + length, deadline)
        payload = bytes(self.buffered[header_length : header_length + length])
        del self.buffered[: header_length + length]
        return first, second, payload

    def send_frame(self, opcode, payload=b"", fin=True):
        require(isinstance(payload, bytes), "WebSocket payload must be bytes")
        require(opcode in {0x1, 0x8, 0x9, 0xA}, "unsupported outbound WebSocket opcode")
        if opcode >= 0x8:
            require(fin and len(payload) <= 125, "invalid WebSocket control frame")
        require(len(payload) <= MAX_FRAME, "outbound WebSocket frame exceeded limit")
        first = (0x80 if fin else 0) | opcode
        length = len(payload)
        if length <= 125:
            header = bytes([first, 0x80 | length])
        elif length <= 0xFFFF:
            header = bytes([first, 0x80 | 126]) + struct.pack("!H", length)
        else:
            header = bytes([first, 0x80 | 127]) + struct.pack("!Q", length)
        mask = os.urandom(4)
        encoded = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
        self.stream.sendall(header + mask + encoded)

    def send_json(self, value):
        self.send_frame(0x1, json.dumps(value, separators=(",", ":"), allow_nan=False).encode("utf-8"))

    def receive_text(self, timeout):
        deadline = time.monotonic() + timeout
        while True:
            first, second, payload = self._read_frame(deadline)
            fin, rsv, opcode, masked = bool(first & 0x80), first & 0x70, first & 0x0F, bool(second & 0x80)
            require(rsv == 0, "WebSocket reserved bits are unsupported")
            require(not masked, "server WebSocket frame was masked")
            length = len(payload)
            if opcode >= 0x8:
                require(fin and length <= 125, "fragmented or oversized WebSocket control frame")
            if opcode == 0x9:
                self.send_frame(0xA, payload)
                continue
            if opcode == 0xA:
                continue
            if opcode == 0x8:
                self._validate_close_payload(payload)
                if not self.close_sent:
                    self.send_frame(0x8, payload)
                    self.close_sent = True
                raise Closed("browser closed the CDP WebSocket")
            require(opcode in {0x0, 0x1}, "CDP WebSocket emitted a non-text message")
            if opcode == 0x1:
                require(self.fragment_opcode is None, "new WebSocket message during fragmentation")
                if fin:
                    data = payload
                else:
                    self.fragment_opcode = opcode
                    self.fragments = bytearray(payload)
                    continue
            else:
                require(self.fragment_opcode == 0x1, "orphan WebSocket continuation frame")
                self.fragments.extend(payload)
                require(len(self.fragments) <= MAX_FRAME, "WebSocket message exceeded limit")
                if not fin:
                    continue
                data = bytes(self.fragments)
                self.fragment_opcode = None
                self.fragments.clear()
            try:
                return data.decode("utf-8")
            except UnicodeDecodeError as error:
                raise TransportError("CDP WebSocket emitted invalid UTF-8") from error

    @staticmethod
    def _validate_close_payload(payload):
        require(len(payload) != 1, "invalid WebSocket close payload")
        if len(payload) < 2:
            return
        code = struct.unpack("!H", payload[:2])[0]
        require(
            code in {1000, 1001, 1002, 1003, 1007, 1008, 1009, 1010, 1011}
            or 3000 <= code <= 4999,
            "invalid WebSocket close status",
        )
        try:
            payload[2:].decode("utf-8")
        except UnicodeDecodeError as error:
            raise TransportError("invalid WebSocket close reason") from error

    def close(self):
        if not self.close_sent:
            try:
                self.send_frame(0x8, struct.pack("!H", 1000))
            except OSError:
                pass
            self.close_sent = True
        self.stream.close()


def category_for_url(value):
    if not isinstance(value, str):
        return None
    try:
        return PATHS.get(urlsplit(value).path)
    except ValueError:
        return None


class Collector:
    def __init__(self, websocket, attach_mode="recursive"):
        require(attach_mode in ATTACH_MODES, "invalid CDP attach mode")
        self.websocket = websocket
        self.attach_mode = attach_mode
        self.next_id = 1
        self.pending = {}
        self.sessions = {}
        self.session_targets = {}
        self.target_owners = {}
        self.target_types = {}
        self.owner_detached_counts = dict.fromkeys(
            sorted(RECEIPT_TARGET_TYPES | {"other"}), 0
        )
        self.resume_after = {}
        self.attached_counts = dict.fromkeys(sorted(RECEIPT_TARGET_TYPES | {"other"}), 0)
        self.requests = {}
        self.counts = dict.fromkeys(CATEGORIES, 0)
        self.finished = dict.fromkeys(CATEGORIES, 0)
        self.failed = dict.fromkeys(CATEGORIES, 0)
        self.encoded_bytes = dict.fromkeys(CATEGORIES, 0.0)
        self.session_types = {category: set() for category in CATEGORIES}
        self.messages = 0

    def command(self, method, params=None, session_id=None, setup=False):
        identifier = self.next_id
        self.next_id += 1
        message = {"id": identifier, "method": method, "params": params or {}}
        if session_id is not None:
            message["sessionId"] = session_id
        target_type = self.sessions.get(session_id, "browser")
        self.pending[identifier] = {
            "method": method,
            "setup": setup,
            "target_type": target_type if target_type in RECEIPT_TARGET_TYPES else "other",
        }
        self.websocket.send_json(message)
        return identifier

    def configure(self, session_id, target_type, target_id):
        sanitized_type = target_type if target_type in RECEIPT_TARGET_TYPES else "other"
        self.sessions[session_id] = sanitized_type
        self.session_targets[session_id] = target_id
        previous_type = self.target_types.setdefault(target_id, sanitized_type)
        require(previous_type == sanitized_type, "CDP target type changed across attachments")
        self.target_owners.setdefault(target_id, session_id)
        self.attached_counts[sanitized_type] += 1
        if target_type in TARGET_TYPES:
            network = self.command("Network.enable", session_id=session_id, setup=True)
            if self.attach_mode == "recursive":
                auto_attach = self.command(
                    "Target.setAutoAttach",
                    {"autoAttach": True, "waitForDebuggerOnStart": True, "flatten": True},
                    session_id=session_id,
                    setup=True,
                )
                prerequisites = {network, auto_attach}
            else:
                prerequisites = {network}
            # Chrome can leave Network.enable unacknowledged while a new service
            # worker is paused, creating a cycle in which the command needs the
            # worker to run before the collector will resume it. Keep that
            # command pending and visible, but do not make it a resume
            # prerequisite. Coverage still requires the synthetic request to
            # finish and every setup command to settle.
            if target_type == "service_worker":
                prerequisites.discard(network)
            if prerequisites:
                self.resume_after[session_id] = prerequisites
            else:
                self.command("Runtime.runIfWaitingForDebugger", session_id=session_id, setup=True)
        else:
            self.command("Runtime.runIfWaitingForDebugger", session_id=session_id, setup=True)

    def start_related(self, result):
        require(isinstance(result, dict), "Target.getTargets result must be an object")
        targets = result.get("targetInfos")
        require(isinstance(targets, list) and len(targets) <= MAX_TARGETS, "invalid target inventory")
        pages = []
        for target in targets:
            require(isinstance(target, dict), "invalid target inventory row")
            target_type, target_id = target.get("type"), target.get("targetId")
            require(isinstance(target_type, str), "invalid target inventory type")
            require(isinstance(target_id, str) and target_id, "invalid target inventory id")
            if target_type == "page":
                pages.append(target_id)
        require(len(pages) == 1, "related attach requires exactly one disposable page target")
        self.command(
            "Target.autoAttachRelated",
            {"targetId": pages[0], "waitForDebuggerOnStart": True},
            setup=True,
        )

    def handle(self, message):
        self.messages += 1
        require(self.messages <= MAX_MESSAGES, "CDP message count exceeded limit")
        require(isinstance(message, dict), "CDP message must be an object")
        if "id" in message:
            identifier = message["id"]
            pending = self.pending.pop(identifier, None)
            require(pending is not None, "CDP returned an unknown command id")
            require("error" not in message, f'CDP command failed: {pending["method"]}')
            if pending["method"] == "Target.getTargets":
                self.start_related(message.get("result"))
            for session_id, prerequisites in list(self.resume_after.items()):
                prerequisites.discard(identifier)
                if not prerequisites:
                    del self.resume_after[session_id]
                    self.command(
                        "Runtime.runIfWaitingForDebugger", session_id=session_id, setup=True
                    )
            return
        method, params = message.get("method"), message.get("params", {})
        require(isinstance(params, dict), "CDP event params must be an object")
        if method == "Target.attachedToTarget":
            session_id = params.get("sessionId")
            target_info = params.get("targetInfo")
            require(isinstance(target_info, dict), "invalid attached target")
            target_type = target_info.get("type")
            target_id = target_info.get("targetId")
            require(
                isinstance(session_id, str)
                and isinstance(target_type, str)
                and isinstance(target_id, str)
                and target_id,
                "invalid attached target",
            )
            self.configure(session_id, target_type, target_id)
            return
        if method == "Target.detachedFromTarget":
            session_id = params.get("sessionId")
            require(isinstance(session_id, str), "invalid detached target")
            target_id = self.session_targets.get(session_id)
            if target_id is not None and self.target_owners.get(target_id) == session_id:
                self.owner_detached_counts[self.sessions[session_id]] += 1
            return
        session_id = message.get("sessionId", "browser")
        target_id = self.session_targets.get(session_id)
        if target_id is not None and self.target_owners[target_id] != session_id:
            return
        request_id = params.get("requestId")
        key = (session_id, request_id)
        if method == "Network.requestWillBeSent":
            category = category_for_url(params.get("request", {}).get("url"))
            if category is None:
                return
            self.counts[category] += 1
            require(self.counts[category] <= MAX_CATEGORY_REQUESTS, "category request count exceeded limit")
            require(isinstance(request_id, str), "categorized CDP request had no id")
            self.requests[key] = category
            self.session_types[category].add(self.sessions.get(session_id, "browser"))
        elif method == "Network.loadingFinished" and key in self.requests:
            category = self.requests.pop(key)
            encoded = params.get("encodedDataLength")
            require(
                isinstance(encoded, (int, float)) and not isinstance(encoded, bool)
                and math.isfinite(encoded) and 0 <= encoded <= (1 << 30),
                "invalid encoded byte count",
            )
            self.finished[category] += 1
            self.encoded_bytes[category] += encoded
        elif method == "Network.loadingFailed" and key in self.requests:
            category = self.requests.pop(key)
            self.failed[category] += 1

    def receive(self, timeout):
        raw = self.websocket.receive_text(timeout)
        try:
            message = json.loads(raw)
        except json.JSONDecodeError as error:
            raise TransportError("CDP WebSocket emitted invalid JSON") from error
        self.handle(message)

    def begin(self):
        if self.attach_mode == "auto-attach-related":
            return self.command("Target.getTargets", setup=True)
        return self.command(
            "Target.setAutoAttach",
            {"autoAttach": True, "waitForDebuggerOnStart": True, "flatten": True},
            setup=True,
        )

    def start(self, deadline):
        root = self.begin()
        quiet_since = None
        while True:
            if root not in self.pending and not any(item["setup"] for item in self.pending.values()):
                quiet_since = quiet_since or time.monotonic()
                if time.monotonic() - quiet_since >= SETUP_SETTLE_SECONDS:
                    return
            else:
                quiet_since = None
            remaining = deadline - time.monotonic()
            require(remaining > 0, "CDP setup timed out")
            try:
                self.receive(min(0.1, remaining))
                quiet_since = None
            except TimeoutError:
                pass

    def collect_until(self, stop_path, deadline):
        stopped_at = None
        quiet_since = None
        while True:
            now = time.monotonic()
            require(now < deadline, "CDP collection timed out")
            if stop_path.exists() and stopped_at is None:
                stopped_at = now
                quiet_since = now
            if stopped_at is not None and now - stopped_at >= STOP_DRAIN_SECONDS:
                return
            try:
                self.receive(min(0.1, deadline - now))
                if stopped_at is not None:
                    quiet_since = time.monotonic()
            except TimeoutError:
                if stopped_at is None:
                    continue
                now = time.monotonic()
                complete = not self.requests and not any(item["setup"] for item in self.pending.values())
                if complete and now - quiet_since >= STOP_QUIET_SECONDS:
                    return

    def report(self):
        pending_counts = dict.fromkeys(CATEGORIES, 0)
        for category in self.requests.values():
            pending_counts[category] += 1
        complete = {
            category: self.counts[category] > 0
            and self.finished[category] == self.counts[category]
            and self.failed[category] == 0
            and pending_counts[category] == 0
            for category in CATEGORIES
        }
        transport_settled = not self.requests and not any(
            item["setup"] for item in self.pending.values()
        )
        ownership_intact = not any(self.owner_detached_counts.values())
        pending_setup_counts = {}
        for item in self.pending.values():
            if not item["setup"]:
                continue
            method = item["method"] if item["method"] in SETUP_METHODS else "other"
            label = f'{item["target_type"]}:{method}'
            pending_setup_counts[label] = pending_setup_counts.get(label, 0) + 1
        distinct_target_counts = dict.fromkeys(sorted(RECEIPT_TARGET_TYPES | {"other"}), 0)
        for target_type in self.target_types.values():
            distinct_target_counts[target_type] += 1
        return {
            "schema_version": SCHEMA,
            "transport": "direct-browser-cdp-flattened-autoattach",
            "attach_mode": self.attach_mode,
            "request_counts": self.counts,
            "finished_counts": self.finished,
            "failed_counts": self.failed,
            "pending_counts": pending_counts,
            "encoded_bytes": self.encoded_bytes,
            "session_types": {category: sorted(values) for category, values in self.session_types.items()},
            "attached_target_counts": self.attached_counts,
            "distinct_target_counts": distinct_target_counts,
            "owner_detached_counts": self.owner_detached_counts,
            "pending_setup_counts": pending_setup_counts,
            "coverage": {
                **complete,
                "transport_settled": transport_settled,
                "ownership_intact": ownership_intact,
                "all_categories_complete": (
                    all(complete.values()) and transport_settled and ownership_intact
                ),
            },
            "limits": {
                "messages": MAX_MESSAGES,
                "frame_bytes": MAX_FRAME,
                "requests_per_category": MAX_CATEGORY_REQUESTS,
            },
        }


def run(args):
    output, ready, stop = map(lambda value: Path(value).resolve(), (args.output, args.ready_file, args.stop_file))
    require(not output.exists() and not ready.exists() and not stop.exists(), "output, ready, and stop paths must be new")
    require(1 <= args.timeout_seconds <= 600, "timeout must be between 1 and 600 seconds")
    require(args.attach_mode in ATTACH_MODES, "invalid CDP attach mode")
    endpoint = discover_websocket(args.debugger_address)
    websocket = WebSocket.connect(endpoint)
    try:
        collector = Collector(websocket, args.attach_mode)
        deadline = time.monotonic() + args.timeout_seconds
        collector.start(deadline)
        ready.parent.mkdir(parents=True, exist_ok=True)
        with ready.open("x") as stream:
            stream.write("ready\n")
        collector.collect_until(stop, deadline)
        report = collector.report()
        output.parent.mkdir(parents=True, exist_ok=True)
        with output.open("x") as stream:
            json.dump(report, stream, indent=2, sort_keys=True, allow_nan=False)
            stream.write("\n")
        return 0 if report["coverage"]["all_categories_complete"] else 2
    finally:
        websocket.close()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--debugger-address", required=True, help="loopback Chrome debugger host:port")
    parser.add_argument("--output", required=True, help="new sanitized JSON receipt")
    parser.add_argument("--ready-file", required=True, help="new readiness signal path")
    parser.add_argument("--stop-file", required=True, help="collector stops after this path appears")
    parser.add_argument("--timeout-seconds", type=int, default=60)
    parser.add_argument(
        "--attach-mode",
        choices=sorted(ATTACH_MODES),
        default=os.environ.get("TONK_PERF_CDP_ATTACH_MODE", "recursive"),
    )
    args = parser.parse_args(argv)
    try:
        return run(args)
    except (TransportError, Closed, OSError, ValueError, KeyError, TypeError) as error:
        print(f"INVALID: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
