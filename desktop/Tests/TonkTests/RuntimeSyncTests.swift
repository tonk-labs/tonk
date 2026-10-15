import WebKit
import XCTest

@testable import Tonk

@MainActor
final class RuntimeSyncTests: XCTestCase {
  func testPullPreservesDocumentAndDraftAndRejectsUnconfirmedResult() async throws {
    let webView = WKWebView()
    webView.loadHTMLString("<input id='draft' value='unsaved input'>", baseURL: nil)
    for _ in 0..<100 {
      if !webView.isLoading { break }
      try await Task.sleep(for: .milliseconds(20))
    }
    let script = """
      const original = document.getElementById('draft');
      original.focus();
      async function api(path, body) {
        window.request = {path, body};
        return {success: succeeds};
      }
      async function pull() { \(RuntimeModel.pullSpaceScript) }
      let rejected = false;
      try { await pull(); } catch { rejected = true; }
      return {
        rejected, path: window.request.path, body: window.request.body,
        sameNode: original === document.getElementById('draft'),
        focused: document.activeElement === original, value: original.value
      };
      """
    for succeeds in [true, false] {
      let result =
        try await webView.callAsyncJavaScript(
          script, arguments: ["subject": "did:key:z123", "succeeds": succeeds],
          in: nil, contentWorld: .page) as! [String: Any]
      XCTAssertEqual(
        result["path"] as? String, "/api/repository/did%3Akey%3Az123/branch/main/sync/pull")
      XCTAssertEqual(result["rejected"] as? Bool, !succeeds)
      XCTAssertEqual(result["sameNode"] as? Bool, true)
      XCTAssertEqual(result["focused"] as? Bool, true)
      XCTAssertEqual(result["value"] as? String, "unsaved input")
      XCTAssertEqual((result["body"] as? [String: Any])?.count, 0)
    }
  }
}
