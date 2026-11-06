import WebKit
import XCTest

@testable import Tonk

@MainActor
final class RuntimeSignOutTests: XCTestCase {
  func testDeviceSignOutUsesUnlinkStatusWithoutRequestingFromRetiredPage() async throws {
    let webView = WKWebView()
    webView.loadHTMLString("<p>Account fixture</p>", baseURL: nil)
    for _ in 0..<100 {
      if !webView.isLoading { break }
      try await Task.sleep(for: .milliseconds(20))
    }
    let script = """
      const requests = [];
      const fetch = async (path, options) => {
        requests.push({path, method: options.method});
        return {ok: succeeds, status: succeeds ? 200 : 503, json: async () => ({status})};
      };
      const api = async path => {
        requests.push({path, method: 'GET'});
        throw new Error(path + ' failed (HTTP 409): profile changed; reload required');
      };
      let signedOut = false;
      let rejected = false;
      try {
        signedOut = (await (async () => { \(RuntimeModel.signOutScript) })()).signedOut;
      } catch { rejected = true; }
      return {requests, signedOut, rejected};
      """
    for (status, succeeds, expected) in [
      ("rootMissing", true, true), ("unregistered", true, true),
      ("registered", true, false), ("unknown", true, false), ("rootMissing", false, false),
    ] {
      let result =
        try await webView.callAsyncJavaScript(
          script, arguments: ["status": status, "succeeds": succeeds], in: nil, contentWorld: .page
        ) as! [String: Any]
      XCTAssertEqual(result["signedOut"] as? Bool, expected)
      XCTAssertEqual(result["rejected"] as? Bool, !expected)
      let requests = result["requests"] as! [[String: String]]
      XCTAssertEqual(requests.first, ["path": "/api/account", "method": "DELETE"])
      XCTAssertEqual(requests.count, 1)
    }
  }
}
