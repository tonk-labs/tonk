import WebKit
import XCTest

@testable import Tonk

@MainActor
final class RuntimeAccountTests: XCTestCase {
  func testProfileRoutingDetectsWorkerAPIWithoutRetryingWrites() async throws {
    let webView = WKWebView()
    webView.loadHTMLString("<p>Routing fixture</p>", baseURL: nil)
    for _ in 0..<100 {
      if !webView.isLoading { break }
      try await Task.sleep(for: .milliseconds(20))
    }
    for status in [200, 404, 500] {
      let result = try await webView.callAsyncJavaScript(
        """
        const requests = [];
        const fetch = async (path, options) => {
          requests.push({path, method: options.method || 'GET', body: options.body || null});
          const code = path === '/api/profile/repository' ? probeStatus : 200;
          return {ok: code === 200, status: code,
            headers: new Headers({'content-type': 'application/json'}), json: async () => ({})};
        };
        \(RuntimeModel.accountAPIScript)
        let failed = false;
        try {
          await api('/api/profile/branch/account%2Fone/transact', {changes: []});
          await api('/api/profile/branch/meta/query', {query: true});
          await api('/api/account');
        } catch { failed = true; }
        return {requests, failed};
        """, arguments: ["probeStatus": status], in: nil, contentWorld: .page)
      let resultObject = try XCTUnwrap(result as? [String: Any])
      let requests = try XCTUnwrap(resultObject["requests"] as? [[String: Any]])
      XCTAssertEqual(requests.first?["path"] as? String, "/api/profile/repository")
      XCTAssertEqual(requests.first?["method"] as? String, "GET")
      XCTAssertEqual(resultObject["failed"] as? Bool, status == 500)
      if status == 500 {
        XCTAssertEqual(requests.count, 1, "A failed probe must not issue a write")
        continue
      }
      let prefix = status == 200 ? "/api/profile/branch/" : "/api/repository/profile:tonk/branch/"
      XCTAssertEqual(requests.map { $0["path"] as? String }, [
        "/api/profile/repository", prefix + "account%2Fone/transact",
        prefix + "meta/query", "/api/account",
      ])
      XCTAssertEqual(requests.map { $0["method"] as? String }, ["GET", "POST", "POST", "GET"])
      XCTAssertEqual(requests[1]["body"] as? String, "{\"changes\":[]}")
      XCTAssertEqual(requests[2]["body"] as? String, "{\"query\":true}")
    }
  }
}
