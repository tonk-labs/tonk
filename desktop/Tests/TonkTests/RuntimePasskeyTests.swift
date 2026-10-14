import XCTest
import WebKit

@testable import Tonk

@MainActor
final class RuntimePasskeyTests: XCTestCase {
  func testPreflightKeepsExistingRootOnConstrainedBrowserPath() async throws {
    let webView = try await fixture()
    for (root, status, expected) in [
      ("", "rootMissing", "ready"),
      ("did:key:existing", "unregistered", "browserRequired"),
      ("did:key:existing", "registered", "connected"),
    ] {
      let result = try await webView.callAsyncJavaScript(
        """
        window.tonkIdentity = {usePasskey() { throw new Error('Preflight must not assert'); }};
        const api = async path => path === '/api/identity/root'
          ? {deviceDid: 'did:key:device', rootDid: rootValue} : {status};
        \(RuntimeModel.passkeyPreflightScript)
        """, arguments: ["rootValue": root, "status": status], in: nil, contentWorld: .page
      ) as! [String: Any]
      XCTAssertEqual(result[expected] as? Bool, true)
    }
  }

  func testLoginUsesWorkerCustodyWithoutReturningSecretMaterial() async throws {
    let webView = try await fixture()
    let result = try await webView.callAsyncJavaScript(
      """
      let received;
      window.tonkIdentity = {usePasskey(input) {
        received = input;
        return Promise.resolve({reload: true, privateValue: 'must remain in WebKit'});
      }};
      const result = await (async () => { \(RuntimeModel.passkeyLoginScript) })();
      return {received, result};
      """, arguments: ["runtimeOrigin": "https://tonk.network"], in: nil, contentWorld: .page
    ) as! [String: Any]
    let received = result["received"] as! [String: Any]
    let request = received["request"] as! [String: String]
    XCTAssertEqual(request, ["kind": "login", "deviceName": "Tonk",
      "endpoint": "https://tonk.network/ucan/", "provider": "https://tonk.network/ucan/"])
    let completion = result["result"] as! [String: Bool]
    XCTAssertEqual(completion, ["accepted": true, "reload": true])
  }

  private func fixture() async throws -> WKWebView {
    let webView = WKWebView()
    webView.loadHTMLString("<p>Passkey login fixture</p>", baseURL: nil)
    for _ in 0..<100 {
      if !webView.isLoading { return webView }
      try await Task.sleep(for: .milliseconds(20))
    }
    throw NSError(domain: "PasskeyTest", code: 1)
  }
}
