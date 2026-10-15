import Foundation
import XCTest

@testable import HarnessCore

final class APIHTTPTests: XCTestCase {
  // Real URLSession/SSE coverage without credentials or paid provider requests.
  @MainActor
  func testHTTPStreamingToolsErrorsRedirectsAndCancellation() async throws {
    let server = Process()
    server.executableURL = URL(fileURLWithPath: "/usr/bin/python3")
    server.arguments = ["-u", "-c", Self.server]
    let output = Pipe()
    server.standardOutput = output
    server.standardError = FileHandle.nullDevice
    try server.run()
    defer {
      server.terminate()
      server.waitUntilExit()
    }
    let line = String(decoding: output.fileHandleForReading.availableData, as: UTF8.self)
      .trimmingCharacters(in: .whitespacesAndNewlines)
    let port = try XCTUnwrap(Int(line))
    let base = "http://127.0.0.1:\(port)"
    var history: [APIMessage] = []
    var text = ""
    var calls = 0
    try await APIModelClient().run(
      connection: ModelConnection(provider: .local, baseURL: base + "/v1", model: "fixture"),
      key: "", instructions: "Test", history: [APIMessage(role: "user", text: "Inspect")],
      tools: SpaceTools.agentDefinitions(includeCLI: false), onHistory: { history = $0 },
      onText: { _, delta in text += delta },
      execute: { call in
        calls += 1
        XCTAssertEqual(call.name, "tonk_guide")
        return SpaceTools.response("Checked", success: true)
      })
    XCTAssertEqual(calls, 1)
    XCTAssertEqual(text, "Ready ✓")
    XCTAssertEqual(history.last?.text, "Ready ✓")
    var request = URLRequest(url: URL(string: base + "/anthropic")!)
    request.httpMethod = "POST"
    let anthropic = try await APIModelClient.fetch(request, anthropic: true) { _ in }
    XCTAssertEqual(anthropic.text, "Claude fixture")
    for path in ["error", "redirect", "truncated"] {
      request.url = URL(string: base + "/" + path)!
      do {
        _ = try await APIModelClient.fetch(request, anthropic: false) { _ in }
        XCTFail("Expected rejection of \(path)")
      } catch {}
    }
    request.url = URL(string: base + "/stall")!
    let stalledRequest = request
    let start = Date()
    let task = Task { try await APIModelClient.fetch(stalledRequest, anthropic: false) { _ in } }
    try await Task.sleep(nanoseconds: 200_000_000)
    task.cancel()
    do {
      _ = try await task.value
      XCTFail("Expected cancellation")
    } catch {}
    XCTAssertLessThan(
      Date().timeIntervalSince(start), 3, "Stop must cancel a stalled network stream promptly")
  }

  private static let server = #"""
    import http.server, json, time
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_POST(self):
            data = self.rfile.read(int(self.headers.get('Content-Length', 0)))
            if self.path == '/error':
                self.send_response(401); self.end_headers(); return
            if self.path == '/redirect':
                self.send_response(307); self.send_header('Location', '/v1/chat/completions'); self.end_headers(); return
            self.send_response(200); self.send_header('Content-Type', 'text/event-stream'); self.end_headers()
            def event(value):
                payload = value if isinstance(value, str) else json.dumps(value)
                self.wfile.write(('data: ' + payload + '\n\n').encode()); self.wfile.flush()
            if self.path == '/stall':
                self.wfile.write(b': connected\n\n'); self.wfile.flush(); time.sleep(6); return
            if self.path == '/anthropic':
                event({'type':'content_block_delta','index':0,'delta':{'type':'text_delta','text':'Claude fixture'}})
                event({'type':'message_delta','delta':{'stop_reason':'end_turn'}})
                event({'type':'message_stop'}); return
            if self.path == '/truncated':
                event({'choices':[{'delta':{'content':'Incomplete'}}]}); return
            body = json.loads(data)
            if body['messages'][-1]['role'] == 'tool':
                assert body['messages'][-1]['tool_call_id'] == 'fixture-call'
                event({'choices':[{'delta':{'content':'Ready '}}]})
                event({'choices':[{'delta':{'content':'✓'},'finish_reason':'stop'}]})
            else:
                event({'choices':[{'delta':{'tool_calls':[{'index':0,'id':'fixture-call','function':{'name':'tonk_guide','arguments':'{'}}]}}]})
                event({'choices':[{'delta':{'tool_calls':[{'index':0,'function':{'arguments':'}'}}]},'finish_reason':'tool_calls'}]})
            event('[DONE]')
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    print(server.server_port, flush=True)
    server.serve_forever()
    """#
}
