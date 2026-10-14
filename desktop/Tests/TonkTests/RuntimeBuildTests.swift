import HarnessCore
import WebKit
import XCTest

@testable import Tonk

@MainActor
final class RuntimeBuildTests: XCTestCase {
  func testCanonicalWorkerScriptInWebKitPreservesDraftAndNeverReplaysWrites() async throws {
    let webView = WKWebView()
    webView.loadHTMLString("<input id='draft' value='keep'>", baseURL: nil)
    for _ in 0..<100 {
      if !webView.isLoading { break }
      try await Task.sleep(for: .milliseconds(20))
    }
    let script = """
      let requests = [];
      async function fetch(path, options) {
        requests.push({path, body: options.body});
        if (lost) throw new Error('Lost reply');
        return new Response(JSON.stringify({revision_before: null, revision_after: null,
          commits: {claims: 0}, matches_after: []}), {headers: {'content-type': 'application/json'}});
      }
      async function invoke() { \(TonkAgentContext.workerScript) }
      const result = await invoke();
      return {requests, result, draft: window.document.getElementById('draft').value};
      """
    for tool in ["tonk_query", "tonk_evaluate"] {
      for lost in [false, true] {
        let result = try await webView.callAsyncJavaScript(
          script, arguments: ["subject": "did:key:z123", "tool": tool,
            "argumentsJSON": "{\"document\":\"concept:\"}", "lost": lost],
          in: nil, contentWorld: .page) as! [String: Any]
        let requests = result["requests"] as! [[String: Any]]
        XCTAssertEqual(requests.count, 1)
        XCTAssertEqual(requests[0]["path"] as? String,
          "/api/repository/did%3Akey%3Az123/branch/main/evaluate?transact=\(tool == "tonk_evaluate")")
        XCTAssertEqual(requests[0]["body"] as? String, "concept:")
        XCTAssertEqual(result["draft"] as? String, "keep")
        let outcome = result["result"] as! [String: Any]
        if lost && tool == "tonk_evaluate" {
          XCTAssertTrue((outcome["error"] as? String)?.contains("Do not repeat") == true)
        } else if !lost { XCTAssertNotNil(outcome["response"]) }
      }
    }
  }
}
