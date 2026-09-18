import base64
import hashlib
import json
import struct
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

import cdp_transport as cdp


class FakeSocket:
    def __init__(self, incoming=b""):
        self.incoming = bytearray(incoming)
        self.sent = bytearray()
        self.closed = False

    def settimeout(self, _timeout):
        pass

    def recv(self, length):
        if not self.incoming:
            return b""
        chunk = bytes(self.incoming[:length])
        del self.incoming[:length]
        return chunk

    def sendall(self, value):
        self.sent.extend(value)

    def close(self):
        self.closed = True


class ScriptedSocket(FakeSocket):
    def __init__(self, actions):
        super().__init__()
        self.actions = list(actions)

    def recv(self, length):
        action = self.actions.pop(0)
        if isinstance(action, BaseException):
            raise action
        assert len(action) <= length
        return action


def server_frame(opcode, payload=b"", fin=True, masked=False):
    first = (0x80 if fin else 0) | opcode
    length = len(payload)
    assert length <= 125
    second = (0x80 if masked else 0) | length
    if not masked:
        return bytes([first, second]) + payload
    mask = b"mask"
    encoded = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
    return bytes([first, second]) + mask + encoded


def decode_client_frame(value):
    first, second = value[:2]
    assert second & 0x80
    length = second & 0x7F
    offset = 2
    if length == 126:
        length = struct.unpack("!H", value[offset : offset + 2])[0]
        offset += 2
    elif length == 127:
        length = struct.unpack("!Q", value[offset : offset + 8])[0]
        offset += 8
    mask = value[offset : offset + 4]
    payload = value[offset + 4 : offset + 4 + length]
    return first & 0x0F, bytes(byte ^ mask[index % 4] for index, byte in enumerate(payload))


class WebSocketTests(unittest.TestCase):
    def test_handshake_verifies_rfc_accept(self):
        key = "dGhlIHNhbXBsZSBub25jZQ=="
        accept = base64.b64encode(hashlib.sha1((key + cdp.GUID).encode()).digest()).decode()
        headers = (
            "HTTP/1.1 101 Switching Protocols\r\n"
            "Upgrade: websocket\r\nConnection: keep-alive, Upgrade\r\n"
            f"Sec-WebSocket-Accept: {accept}"
        ).encode()
        cdp.WebSocket._verify_handshake(headers, key)
        with self.assertRaises(cdp.TransportError):
            cdp.WebSocket._verify_handshake(headers.replace(accept.encode(), b"wrong"), key)

    def test_fragmentation_and_interleaved_ping(self):
        incoming = (
            server_frame(0x1, b'{"ok":', fin=False)
            + server_frame(0x9, b"probe")
            + server_frame(0x0, b"true}")
        )
        stream = FakeSocket(incoming)
        websocket = cdp.WebSocket(stream)
        self.assertEqual(websocket.receive_text(1), '{"ok":true}')
        self.assertEqual(decode_client_frame(bytes(stream.sent)), (0xA, b"probe"))

    def test_client_frames_are_masked_and_server_frames_must_not_be(self):
        stream = FakeSocket()
        websocket = cdp.WebSocket(stream)
        websocket.send_json({"id": 1})
        self.assertTrue(stream.sent[1] & 0x80)
        self.assertEqual(decode_client_frame(bytes(stream.sent)), (0x1, b'{"id":1}'))
        with self.assertRaises(cdp.TransportError):
            cdp.WebSocket(FakeSocket(server_frame(0x1, b"bad", masked=True))).receive_text(1)

    def test_invalid_fragment_and_control_frames_fail(self):
        with self.assertRaises(cdp.TransportError):
            cdp.WebSocket(FakeSocket(server_frame(0x0, b"orphan"))).receive_text(1)
        with self.assertRaises(cdp.TransportError):
            cdp.WebSocket(FakeSocket(server_frame(0x9, b"ping", fin=False))).receive_text(1)

    def test_timeout_preserves_partial_frame(self):
        frame = server_frame(0x1, b'{"ok":true}')
        scripts = [
            [frame[:2], TimeoutError(), frame[2:]],
            [frame[:2], frame[2:5], TimeoutError(), frame[5:]],
        ]
        for actions in scripts:
            with self.subTest(chunks=[type(action).__name__ for action in actions]):
                stream = ScriptedSocket(actions)
                websocket = cdp.WebSocket(stream)
                with self.assertRaises(TimeoutError):
                    websocket.receive_text(1)
                self.assertEqual(websocket.receive_text(1), '{"ok":true}')

    def test_close_status_and_reason_are_validated(self):
        invalid = [b"x", struct.pack("!H", 1005), struct.pack("!H", 1000) + b"\xff"]
        for payload in invalid:
            with self.subTest(payload=payload), self.assertRaises(cdp.TransportError):
                cdp.WebSocket(FakeSocket(server_frame(0x8, payload))).receive_text(1)


class AddressTests(unittest.TestCase):
    def test_only_loopback_debugger_addresses_are_allowed(self):
        self.assertEqual(cdp.parse_debugger_address("127.0.0.1:9222"), ("127.0.0.1", 9222))
        self.assertEqual(cdp.parse_debugger_address("[::1]:9222"), ("::1", 9222))
        self.assertEqual(cdp.parse_debugger_address("localhost:9222"), ("localhost", 9222))
        for value in ["example.com:9222", "0.0.0.0:9222", "127.0.0.1", "127.0.0.1:9222/path"]:
            with self.subTest(value=value), self.assertRaises(cdp.TransportError):
                cdp.parse_debugger_address(value)


class FakeWebSocket:
    def __init__(self):
        self.sent = []

    def send_json(self, value):
        self.sent.append(value)


class DelayedEventWebSocket(FakeWebSocket):
    def __init__(self, messages):
        super().__init__()
        self.messages = list(messages)

    def receive_text(self, _timeout):
        if self.messages:
            message = self.messages.pop(0)
            if isinstance(message, BaseException):
                time.sleep(0.01)
                raise message
            return json.dumps(message)
        time.sleep(0.01)
        raise TimeoutError


class CollectorTests(unittest.TestCase):
    def test_related_mode_bootstraps_exactly_one_disposable_page(self):
        websocket = FakeWebSocket()
        collector = cdp.Collector(websocket, "auto-attach-related")
        inventory = collector.begin()
        self.assertEqual(websocket.sent, [{"id": inventory, "method": "Target.getTargets", "params": {}}])
        collector.handle({
            "id": inventory,
            "result": {"targetInfos": [
                {"type": "page", "targetId": "disposable-page"},
                {"type": "service_worker", "targetId": "existing-worker"},
            ]},
        })
        self.assertEqual(websocket.sent[-1], {
            "id": inventory + 1,
            "method": "Target.autoAttachRelated",
            "params": {"targetId": "disposable-page", "waitForDebuggerOnStart": True},
        })

    def test_related_mode_rejects_ambiguous_page_inventory(self):
        for pages in [[], ["one", "two"]]:
            with self.subTest(pages=pages):
                collector = cdp.Collector(FakeWebSocket(), "auto-attach-related")
                inventory = collector.begin()
                with self.assertRaises(cdp.TransportError):
                    collector.handle({
                        "id": inventory,
                        "result": {"targetInfos": [
                            {"type": "page", "targetId": target_id} for target_id in pages
                        ]},
                    })

    def test_related_mode_enables_network_without_recursive_autoattach(self):
        websocket = FakeWebSocket()
        collector = cdp.Collector(websocket, "auto-attach-related")
        collector.handle({
            "method": "Target.attachedToTarget",
            "params": {
                "sessionId": "iframe-session",
                "targetInfo": {"type": "iframe", "targetId": "iframe-target"},
            },
        })
        self.assertEqual([message["method"] for message in websocket.sent], ["Network.enable"])
        collector.handle({"id": websocket.sent[0]["id"], "result": {}})
        self.assertEqual(websocket.sent[-1]["method"], "Runtime.runIfWaitingForDebugger")

    def test_attached_targets_wait_for_network_and_autoattach_acks_before_resume(self):
        websocket = FakeWebSocket()
        collector = cdp.Collector(websocket)
        collector.handle({
            "method": "Target.attachedToTarget",
            "params": {
                "sessionId": "worker-session",
                "targetInfo": {"type": "worker", "targetId": "worker-target"},
            },
        })
        self.assertEqual(
            [message["method"] for message in websocket.sent],
            ["Network.enable", "Target.setAutoAttach"],
        )
        self.assertTrue(all(message["sessionId"] == "worker-session" for message in websocket.sent))
        self.assertTrue(all(item["setup"] for item in collector.pending.values()))
        collector.handle({"id": websocket.sent[0]["id"], "result": {}})
        self.assertEqual(len(websocket.sent), 2)
        report = collector.report()
        self.assertEqual(report["attached_target_counts"]["worker"], 1)
        self.assertEqual(report["pending_setup_counts"], {"worker:Target.setAutoAttach": 1})
        collector.handle({"id": websocket.sent[1]["id"], "result": {}})
        self.assertEqual(websocket.sent[-1]["method"], "Runtime.runIfWaitingForDebugger")
        self.assertEqual(websocket.sent[-1]["sessionId"], "worker-session")

    def test_service_worker_resumes_after_autoattach_while_network_ack_stays_pending(self):
        websocket = FakeWebSocket()
        collector = cdp.Collector(websocket)
        collector.handle({
            "method": "Target.attachedToTarget",
            "params": {
                "sessionId": "sw-session",
                "targetInfo": {"type": "service_worker", "targetId": "sw-target"},
            },
        })
        network, auto_attach = websocket.sent
        collector.handle({"id": auto_attach["id"], "result": {}})
        self.assertEqual(websocket.sent[-1]["method"], "Runtime.runIfWaitingForDebugger")
        report = collector.report()
        self.assertEqual(
            report["pending_setup_counts"],
            {"service_worker:Network.enable": 1, "service_worker:Runtime.runIfWaitingForDebugger": 1},
        )
        self.assertFalse(report["coverage"]["all_categories_complete"])
        collector.handle({"id": network["id"], "result": {}})
        self.assertEqual(
            collector.report()["pending_setup_counts"],
            {"service_worker:Runtime.runIfWaitingForDebugger": 1},
        )

    def test_duplicate_sessions_for_one_target_have_one_counting_owner(self):
        collector = cdp.Collector(FakeWebSocket())
        for session_id in ["sw-browser", "sw-parent"]:
            collector.handle({
                "method": "Target.attachedToTarget",
                "params": {
                    "sessionId": session_id,
                    "targetInfo": {"type": "service_worker", "targetId": "same-target"},
                },
            })
        request = {
            "method": "Network.requestWillBeSent",
            "params": {"requestId": "same-request", "request": {
                "url": "https://fixture.invalid/__tonk_transport_probe__/service-worker.txt"
            }},
        }
        finished = {
            "method": "Network.loadingFinished",
            "params": {"requestId": "same-request", "encodedDataLength": 58},
        }
        for session_id in ["sw-browser", "sw-parent"]:
            collector.handle({**request, "sessionId": session_id})
            collector.handle({**finished, "sessionId": session_id})
        report = collector.report()
        self.assertEqual(report["attached_target_counts"]["service_worker"], 2)
        self.assertEqual(report["distinct_target_counts"]["service_worker"], 1)
        self.assertEqual(report["request_counts"]["service_worker"], 1)
        self.assertEqual(report["finished_counts"]["service_worker"], 1)
        self.assertEqual(report["encoded_bytes"]["service_worker"], 58)

    def test_owner_detachment_blocks_coverage_instead_of_reassigning(self):
        collector = cdp.Collector(FakeWebSocket())
        collector.handle({
            "method": "Target.attachedToTarget",
            "params": {
                "sessionId": "owner",
                "targetInfo": {"type": "worker", "targetId": "target"},
            },
        })
        collector.handle({
            "method": "Target.detachedFromTarget",
            "params": {"sessionId": "owner", "targetId": "target"},
        })
        report = collector.report()
        self.assertEqual(report["owner_detached_counts"]["worker"], 1)
        self.assertFalse(report["coverage"]["ownership_intact"])
        self.assertFalse(report["coverage"]["all_categories_complete"])

    def test_receipt_is_categorical_and_uses_finished_encoded_bytes(self):
        collector = cdp.Collector(FakeWebSocket())
        examples = [
            ("root", "page"),
            ("dedicated_worker", "worker"),
            ("service_worker", "service_worker"),
            ("opaque_iframe", "page"),
            ("opaque_iframe_late", "page"),
            ("cross_origin_iframe", "iframe"),
        ]
        for index, (category, target_type) in enumerate(examples):
            session = f"session-{index}"
            request = f"request-{index}"
            path = next(path for path, label in cdp.PATHS.items() if label == category)
            collector.sessions[session] = target_type
            collector.handle({
                "sessionId": session,
                "method": "Network.requestWillBeSent",
                "params": {"requestId": request, "request": {
                    "url": f"https://fixture.invalid{path}?secret=ignored"
                }},
            })
            collector.handle({
                "sessionId": session,
                "method": "Network.loadingFinished",
                "params": {"requestId": request, "encodedDataLength": index + 10},
            })
        report = collector.report()
        self.assertTrue(report["coverage"]["all_categories_complete"])
        self.assertEqual(report["encoded_bytes"]["service_worker"], 12)
        serialized = json.dumps(report)
        self.assertNotIn("fixture.invalid", serialized)
        self.assertNotIn("secret", serialized)

    def test_missing_or_failed_category_blocks_complete_coverage(self):
        collector = cdp.Collector(FakeWebSocket())
        report = collector.report()
        self.assertFalse(report["coverage"]["all_categories_complete"])

    def test_partially_completed_category_is_not_complete(self):
        collector = cdp.Collector(FakeWebSocket())
        path = "/__tonk_transport_probe__/root.txt"
        for request in ["finished", "pending"]:
            collector.handle({
                "sessionId": "page",
                "method": "Network.requestWillBeSent",
                "params": {"requestId": request, "request": {"url": f"https://fixture.invalid{path}"}},
            })
        collector.handle({
            "sessionId": "page",
            "method": "Network.loadingFinished",
            "params": {"requestId": "finished", "encodedDataLength": 12},
        })
        report = collector.report()
        self.assertEqual(report["request_counts"]["root"], 2)
        self.assertEqual(report["finished_counts"]["root"], 1)
        self.assertEqual(report["pending_counts"]["root"], 1)
        self.assertFalse(report["coverage"]["root"])
        self.assertFalse(report["coverage"]["all_categories_complete"])

    def test_unknown_command_response_is_invalid(self):
        collector = cdp.Collector(FakeWebSocket())
        with self.assertRaises(cdp.TransportError):
            collector.handle({"id": 99, "result": {}})

    def test_stop_waits_through_quiet_gap_for_queued_events(self):
        path = "/__tonk_transport_probe__/root.txt"
        websocket = DelayedEventWebSocket([
            TimeoutError(),
            {"sessionId": "page", "method": "Network.requestWillBeSent", "params": {
                "requestId": "request", "request": {"url": f"https://fixture.invalid{path}"}
            }},
            {"sessionId": "page", "method": "Network.loadingFinished", "params": {
                "requestId": "request", "encodedDataLength": 12
            }},
        ])
        collector = cdp.Collector(websocket)
        with tempfile.TemporaryDirectory() as directory:
            stop = Path(directory, "stop")
            stop.write_text("stop\n")
            with mock.patch.object(cdp, "STOP_QUIET_SECONDS", 0.03), mock.patch.object(
                cdp, "STOP_DRAIN_SECONDS", 0.2
            ):
                collector.collect_until(stop, time.monotonic() + 1)
        self.assertEqual(collector.counts["root"], 1)
        self.assertEqual(collector.finished["root"], 1)


if __name__ == "__main__":
    unittest.main()
