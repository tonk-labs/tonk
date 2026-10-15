import WebKit
import XCTest

@testable import Tonk

/// Opt-in checks against locally served fixtures. Ordinary test runs need no server.
@MainActor
final class ColdStreamDiagnosticTests: XCTestCase {
  func testInitialServiceWorkerFrames() async throws {
    guard ProcessInfo.processInfo.environment["TONK_STREAM_PROBE"] == "1" else {
      throw XCTSkip("Serve experiments/render-provenance/cold-stream on port 4191")
    }
    let configuration = WKWebViewConfiguration()
    configuration.websiteDataStore = WKWebsiteDataStore(forIdentifier: UUID())
    let webView = WKWebView(frame: .zero, configuration: configuration)
    webView.load(URLRequest(url: URL(string: "http://127.0.0.1:4191/index.html")!))
    for _ in 0..<100 {
      if (try? await webView.evaluateJavaScript("typeof window.probe")) as? String == "object" {
        break
      }
      try await Task.sleep(for: .milliseconds(100))
    }
    let result = try await webView.callAsyncJavaScript(
      "return await window.probe;", arguments: [:], in: nil, contentWorld: .page)
    let trials = try XCTUnwrap(result as? [[String: Any]])
    let combined = trials.filter { $0["mode"] as? String == "combined" }
    XCTAssertEqual(combined.count, 4)
    XCTAssertTrue(combined.allSatisfy { $0["passed"] as? Bool == true })
    // Separate-chunk results are diagnostic, not an assertion that WebKit must fail.
    try saveEvidence(trials)
  }

  func testLocalRuntimeColdCompletion() async throws {
    guard ProcessInfo.processInfo.environment["TONK_LOCAL_RUNTIME_SMOKE"] == "1" else {
      throw XCTSkip("Serve the completion runtime on port 4187")
    }
    let inspector = RuntimeInspection()
    let configuration = WKWebViewConfiguration()
    configuration.websiteDataStore = WKWebsiteDataStore(forIdentifier: UUID())
    inspector.install(on: configuration.userContentController)
    let webView = WKWebView(
      frame: CGRect(x: 0, y: 0, width: 800, height: 600), configuration: configuration)
    webView.load(URLRequest(url: URL(string: "http://127.0.0.1:4187/")!))
    for _ in 0..<300 {
      if (try? await webView.evaluateJavaScript("!!navigator.serviceWorker.controller")) as? Bool
        == true
      {
        break
      }
      try await Task.sleep(for: .milliseconds(100))
    }
    let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
      .deletingLastPathComponent().deletingLastPathComponent()
    let script = try String(
      contentsOf: root.appendingPathComponent("scripts/smoke-local-build.js"), encoding: .utf8)
    let fixture = try await webView.callAsyncJavaScript(
      "return await (\(script))();", arguments: [:], in: nil, contentWorld: .page)
    let prepared = try XCTUnwrap(fixture as? [String: Any])
    let nextURL = try XCTUnwrap(prepared["nextURL"] as? String)
    var evidence: [[String: Any]] = []
    for pass in 0..<3 {
      inspector.reset()
      webView.load(URLRequest(url: URL(string: nextURL)!))
      let observation = try await completion(inspector, webView, text: "Waiting for direct apply")
      evidence.append(
        try await verify(observation, in: webView, phase: pass == 0 ? "cold" : "reload-\(pass)"))
    }
    // This fixture performs preview, conditional apply, stale rejection and readback.
    let applied = try await webView.callAsyncJavaScript(
      "return await (\(script))();", arguments: [:], in: nil, contentWorld: .page)
    XCTAssertEqual((applied as? [String: Any])?["readbackVerified"] as? Bool, true)
    let observation = try await completion(
      inspector, webView, text: "Applied locally without CLI or sync")
    evidence.append(try await verify(observation, in: webView, phase: "edit-without-reload"))
    try saveEvidence(evidence)
  }

  private func completion(_ inspector: RuntimeInspection, _ webView: WKWebView, text: String)
    async throws -> [String: Any]
  {
    var reasons: [String] = []
    for _ in 0..<150 {
      let snapshot = try await inspector.snapshot(in: webView)
      let frames = snapshot["frames"] as? [[String: Any]] ?? []
      let observations = frames.flatMap {
        ($0["renderCompletion"] as? [String: Any])?["observations"] as? [[String: Any]] ?? []
      }
      if let complete = observations.first(where: {
        $0["status"] as? String == "complete" && ($0["textContent"] as? String ?? "").contains(text)
      }) {
        return complete
      }
      reasons = observations.compactMap { $0["reason"] as? String }
      try await Task.sleep(for: .milliseconds(100))
    }
    XCTFail("Missing completion for \(text): \(reasons)")
    throw NSError(domain: "ColdCompletion", code: 1)
  }

  private func verify(_ observation: [String: Any], in webView: WKWebView, phase: String)
    async throws -> [String: Any]
  {
    let query = try await webView.callAsyncJavaScript(
      """
      const subject = JSON.parse(sessionStorage.getItem('tonk-direct-build-smoke')).subject;
      const response = await fetch(`/api/repository/${encodeURIComponent(subject)}/branch/main/evaluate?transact=false`, {
        method: 'POST', headers: {'Content-Type':'text/plain'}, body: 'direct-build-task:\\n'
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return (await response.json()).revision_after;
      """, arguments: [:], in: nil, contentWorld: .page)
    let revision = try XCTUnwrap(query as? [String: Any])
    let displays = try XCTUnwrap(observation["displays"] as? [[String: Any]])
    let checkpoints = displays.flatMap {
      ($0["checkpoints"] as? [String: [String: Any]] ?? [:]).values
    }
    XCTAssertEqual(checkpoints.count, 14)
    let matches = checkpoints.allSatisfy {
      guard let value = $0["revision"] as? [String: Any] else { return false }
      return NSDictionary(dictionary: value).isEqual(to: revision)
    }
    XCTAssertTrue(matches, "Every required checkpoint must match the saved query revision")
    return [
      "phase": phase, "checkpointCount": checkpoints.count,
      "allCheckpointRevisionsMatchQuery": matches,
      "edition": revision["edition"] ?? NSNull(), "textContent": observation["textContent"] ?? "",
    ]
  }

  private func saveEvidence(_ value: Any) throws {
    guard let path = ProcessInfo.processInfo.environment["TONK_COLD_LOAD_EVIDENCE"] else { return }
    try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
      .write(to: URL(fileURLWithPath: path))
  }
}
