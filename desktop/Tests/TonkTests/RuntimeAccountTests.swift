import WebKit
import XCTest

@testable import Tonk

@MainActor
final class RuntimeAccountTests: XCTestCase {
  func testBrowserPreflightStartsBothReadsBeforeEitherCompletes() async throws {
    let webView = try await fixtureWebView()
    let result = try await webView.callAsyncJavaScript(
      """
      const pending = [];
      async function api(path) {
        return new Promise(resolve => {
          pending.push({path, resolve});
          if (pending.length === 2) for (const request of pending) {
            request.resolve(request.path.endsWith('/root')
              ? {deviceDid: 'device', rootDid: 'root'} : {status: 'unregistered'});
          }
        });
      }
      const result = await Promise.race([
        (async () => { \(RuntimeModel.browserSignInPreflightScript) })(),
        new Promise((_, reject) => setTimeout(() => reject(new Error('Reads were serialized')), 1000))
      ]);
      return {result, paths: pending.map(x => x.path)};
      """, arguments: [:], in: nil, contentWorld: .page)
    let object = try XCTUnwrap(result as? [String: Any])
    XCTAssertEqual(object["paths"] as? [String], ["/api/identity/root", "/api/account"])
    let identity = try XCTUnwrap(object["result"] as? [String: String])
    XCTAssertEqual(identity["deviceDid"], "device")
    XCTAssertEqual(identity["rootDid"], "root")
    XCTAssertEqual(identity["status"], "unregistered")
  }

  func testBrowserAuthorizationRefusesChangedProfileBeforeWriting() async throws {
    let webView = try await fixtureWebView()
    for changed in [true, false] {
      let result = try await webView.callAsyncJavaScript(
        """
        const writes = [];
        async function api(path, body) {
          if (body) {
            writes.push(path);
            return path.endsWith('/attach')
              ? {status: 'registered', deviceDid: device}
              : {status: 'ready', deviceDid: device, rootDid: 'authorized-root'};
          }
          return {deviceDid: changed ? 'another-device' : device, rootDid: expectedRoot};
        }
        let failed = false;
        try {
          const root = await (async () => { \(RuntimeModel.browserAuthorizationScript) })();
          const authorizedRoot = root.rootDid;
          await (async () => { \(RuntimeModel.browserAttachmentScript) })();
        } catch { failed = true; }
        return {writes, failed};
        """,
        arguments: [
          "changed": changed, "device": "device", "expectedRoot": "",
          "credential": "fixture", "delegation": "fixture", "remote": "https://example.test",
          "runtimeOrigin": "https://example.test",
        ], in: nil, contentWorld: .page)
      let object = try XCTUnwrap(result as? [String: Any])
      XCTAssertEqual(object["failed"] as? Bool, changed)
      XCTAssertEqual(
        object["writes"] as? [String],
        changed ? [] : ["/api/identity/root", "/api/account/attach"])
    }
  }

  private func fixtureWebView() async throws -> WKWebView {
    let view = WKWebView()
    view.loadHTMLString("<p>Account fixture</p>", baseURL: nil)
    for _ in 0..<100 {
      if !view.isLoading { return view }
      try await Task.sleep(for: .milliseconds(20))
    }
    throw NSError(domain: "AccountFixture", code: 1)
  }

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
      XCTAssertEqual(
        requests.map { $0["path"] as? String },
        [
          "/api/profile/repository", prefix + "account%2Fone/transact",
          prefix + "meta/query", "/api/account",
        ])
      XCTAssertEqual(requests.map { $0["method"] as? String }, ["GET", "POST", "POST", "GET"])
      XCTAssertEqual(requests[1]["body"] as? String, "{\"changes\":[]}")
      XCTAssertEqual(requests[2]["body"] as? String, "{\"query\":true}")
    }
  }
}
