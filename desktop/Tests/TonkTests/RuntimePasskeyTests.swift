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

  func testSignUpCreatesAccountThroughWorkerCustody() async throws {
    let webView = try await fixture()
    let result = try await webView.callAsyncJavaScript(
      """
      let received;
      window.tonkIdentity = {createPasskey(input) {
        received = input;
        return Promise.resolve({ok: {reload: true, privateValue: 'must remain in WebKit'}});
      }};
      const result = await (async () => { \(RuntimeModel.signUpScript) })();
      return {received, result};
      """,
      arguments: [
        "runtimeOrigin": "https://tonk.network", "email": "ada@example.com",
        "displayName": "Ada",
      ], in: nil, contentWorld: .page
    ) as! [String: Any]
    let received = result["received"] as! [String: Any]
    XCTAssertEqual(received["name"] as? String, "ada@example.com")
    XCTAssertEqual(received["displayName"] as? String, "ada@example.com")
    let request = received["request"] as! [String: String]
    XCTAssertEqual(request, [
      "kind": "create-account", "email": "ada@example.com", "displayName": "Ada",
      "deviceName": "Tonk", "remote": "https://tonk.network/ucan/",
      "provider": "https://tonk.network/ucan/", "createdOn": "Tonk on macOS",
    ])
    let completion = result["result"] as! [String: Bool]
    XCTAssertEqual(completion, ["created": true, "reload": true])
  }

  func testSignUpLookupRoutesOnTheAccessServiceAnswer() async throws {
    let webView = try await fixture()
    for (email, status, expected) in [
      ("Ada@Example.com", 404, "unregistered"),
      ("ada@example.com", 200, "active"),
      ("ada@example.com", 202, "pending"),
      ("ada@example.com", 410, "suspended"),
      ("ada@example.com", 429, "unavailable"),
      ("not-an-address", 404, "invalid"),
    ] {
      let result = try await webView.callAsyncJavaScript(
        """
        let requested = null;
        const fetch = async path => { requested = path; return {status}; };
        const answer = await (async () => { \(RuntimeModel.signUpLookupScript) })();
        return {state: answer.state, requested};
        """, arguments: ["email": email, "status": status], in: nil, contentWorld: .page
      ) as! [String: Any]
      XCTAssertEqual(result["state"] as? String, expected, email)
      if expected != "invalid" {
        XCTAssertEqual(result["requested"] as? String, "/customer/example.com/ada/did.json")
      }
    }
  }

  func testSignUpRefusesAddressesTheServiceCannotEnroll() {
    for valid in ["ada@example.com", "ada.lovelace+tonk@mail.example.co.uk"] {
      XCTAssertTrue(RuntimeModel.isPlausibleEmail(valid), valid)
    }
    let long = String(repeating: "a", count: 250) + "@example.com"
    for invalid in [
      "alice@@example.com", "alice@example", "@example.com", "alice@", "alice@.com",
      "alice@example..com", "al ice@example.com", "a/b@example.com", "alice@exa/mple.com", long,
    ] {
      XCTAssertFalse(RuntimeModel.isPlausibleEmail(invalid), invalid)
    }
  }

  func testSignUpProgressTellsAnUnenrolledAccountFromAPendingOne() async throws {
    let webView = try await fixture()
    for (account, customer, expected) in [
      ("rootMissing", "null", "none"),
      ("registered", "null", "unenrolled"),
      ("registered", "'Registered'", "registered"),
      ("registered", "'Active'", "active"),
    ] {
      let result = try await webView.callAsyncJavaScript(
        """
        const api = async path => path === '/api/account'
          ? {status: '\(account)'} : {status: \(customer)};
        return await (async () => { \(RuntimeModel.signUpProgressScript) })();
        """, arguments: [:], in: nil, contentWorld: .page
      ) as! [String: Any]
      XCTAssertEqual(result["state"] as? String, expected)
    }
  }

  func testResumeEnrollmentAssertsTheAccountsOwnPasskey() async throws {
    let webView = try await fixture()
    let result = try await webView.callAsyncJavaScript(
      """
      let received;
      window.tonkIdentity = {usePasskey(input) { received = input; return Promise.resolve({}); }};
      const api = async () => ({rootDid: 'did:key:root', credentialId: 'abcd'});
      const result = await (async () => { \(RuntimeModel.resumeEnrollmentScript) })();
      return {received, result};
      """, arguments: ["email": "ada@example.com"], in: nil, contentWorld: .page
    ) as! [String: Any]
    let received = result["received"] as! [String: Any]
    XCTAssertEqual(received["credentialId"] as? String, "abcd")
    XCTAssertEqual(
      received["request"] as? [String: String], ["kind": "enroll", "email": "ada@example.com"])
    XCTAssertEqual(result["result"] as? [String: Bool], ["enrolled": true])
  }

  func testSettingsShowsTheAccountNameBeforeTheProfileCopy() async throws {
    let webView = try await fixture()
    for (accountName, expected) in [("'Ada'", "Ada"), ("null", "brisk-otter")] {
      let result = try await webView.callAsyncJavaScript(
        """
        const fetch = async () => ({ok: true, json: async () => []});
        const api = async (path, body) => {
          if (path === '/api/profile') return {display_name: 'brisk-otter'};
          if (path === '/api/identity/root') return {rootDid: 'did:key:root'};
          if (path.includes('/meta/')) return [];
          const name = body.predicate.with.name;
          if (name?.the === 'xyz.tonk.account/display-name')
            return \(accountName) === null ? [] : [{fields: {name: \(accountName)}}];
          return [];
        };
        return await (async () => { \(AccessModel.accountScript) })();
        """, arguments: [:], in: nil, contentWorld: .page
      ) as! [String: Any]
      XCTAssertEqual(result["name"] as? String, expected)
    }
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
