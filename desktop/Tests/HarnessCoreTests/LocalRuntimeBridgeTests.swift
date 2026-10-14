import XCTest

@testable import HarnessCore

@MainActor
final class LocalRuntimeBridgeTests: XCTestCase {
  func testInspectionCapabilityRejectsTargetOverrides() async throws {
    var calls = 0
    for enabled in [false, true] {
      let bridge = LocalRuntimeBridge(supportsInspection: enabled) { name, arguments in
        calls += 1
        XCTAssertEqual(name, "tonk_inspect_view")
        XCTAssertEqual(arguments, .object([:]))
        return .object(["frames": .array([]), "revisionTracking": .string("unavailable")])
      }
      let config = try await bridge.start()
      for arguments: JSONValue in [.object([:]), .object(["space": .string("other")])] {
        var request = URLRequest(url: URL(string: config["url"].string! + "/call")!)
        request.httpMethod = "POST"
        request.setValue("application/json", forHTTPHeaderField: "Content-Type")
        request.setValue("Bearer " + config["token"].string!, forHTTPHeaderField: "Authorization")
        request.httpBody = try JSONEncoder().encode(
          JSONValue.object(["name": .string("tonk_inspect_view"), "arguments": arguments]))
        let (data, _) = try await URLSession.shared.data(for: request)
        let value = try JSONDecoder().decode(JSONValue.self, from: data)
        if enabled && arguments == .object([:]) {
          XCTAssertEqual(value["result"]["revisionTracking"], .string("unavailable"))
        } else {
          XCTAssertNotNil(value["error"].string)
        }
      }
      bridge.stop()
    }
    XCTAssertEqual(calls, 1)
  }

  func testGeneralEvaluationUsesTheWriteCapabilityGate() async throws {
    let arguments: JSONValue = .object([
      "document": .string("library/install!:\n  component: tonk:library/notebook\n  time: 1.0\n")
    ])
    var calls = 0
    for enabled in [false, true] {
      let bridge = LocalRuntimeBridge(allowsWrites: enabled) { name, received in
        calls += 1
        XCTAssertEqual(name, "tonk_evaluate")
        XCTAssertEqual(received, arguments)
        return .object(["accepted": .bool(true)])
      }
      let config = try await bridge.start()
      var request = URLRequest(url: URL(string: config["url"].string! + "/call")!)
      request.httpMethod = "POST"
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      request.setValue("Bearer " + config["token"].string!, forHTTPHeaderField: "Authorization")
      request.httpBody = try JSONEncoder().encode(
        JSONValue.object(["name": .string("tonk_evaluate"), "arguments": arguments]))
      let (data, _) = try await URLSession.shared.data(for: request)
      let value = try JSONDecoder().decode(JSONValue.self, from: data)
      if enabled {
        XCTAssertEqual(value["result"]["accepted"], .bool(true))
      } else {
        XCTAssertEqual(value["error"], .string("This runtime connection is read-only."))
      }
      bridge.stop()
    }
    XCTAssertEqual(calls, 1)
  }
  func testAuthenticatedReadOnlyBridgeRejectsBrowserAndUnadvertisedTools() async throws {
    var calls = 0
    let bridge = LocalRuntimeBridge { name, arguments in
      calls += 1
      XCTAssertEqual(name, "tonk_query")
      XCTAssertEqual(arguments["document"], .string("thing:"))
      return .object(["revision": .string("r1"), "committed": .bool(false)])
    }
    let config = try await bridge.start()
    defer { bridge.stop() }
    let base = try XCTUnwrap(config["url"].string)
    let token = try XCTUnwrap(config["token"].string)
    func request(
      _ path: String, body: JSONValue = .object([:]), authorized: Bool = true, origin: Bool = false
    ) async throws -> (Int, JSONValue) {
      var request = URLRequest(url: URL(string: base + path)!)
      request.httpMethod = "POST"
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      if authorized { request.setValue("Bearer " + token, forHTTPHeaderField: "Authorization") }
      if origin { request.setValue("https://example.com", forHTTPHeaderField: "Origin") }
      request.httpBody = try JSONEncoder().encode(body)
      let (data, response) = try await URLSession.shared.data(for: request)
      return (
        (response as! HTTPURLResponse).statusCode,
        try JSONDecoder().decode(JSONValue.self, from: data)
      )
    }
    let denied = try await request("/tools", authorized: false)
    XCTAssertEqual(denied.0, 403)
    let browser = try await request("/tools", origin: true)
    XCTAssertEqual(browser.0, 403)
    let listed = try await request("/tools")
    XCTAssertEqual(
      listed.1["tools"],
      .array([TonkToolCatalog.definition("tonk_query"), TonkToolCatalog.definition("tonk_guide")]))
    let queried = try await request(
      "/call",
      body: .object([
        "name": .string("tonk_query"), "arguments": .object(["document": .string("thing:")]),
      ]))
    XCTAssertEqual(queried.1["result"]["revision"], .string("r1"))
    let guide = try await request(
      "/call",
      body: .object([
        "name": .string("tonk_guide"), "arguments": .object([:]),
      ]))
    XCTAssertEqual(guide.1["result"]["usage"].string, TonkAgentContext.instructions)
    // Guides never dispatch to a worker.
    XCTAssertEqual(calls, 1)
    for name in ["tonk_preview", "tonk_cli", "tonk_space_schema"] {
      let rejected = try await request(
        "/call",
        body: .object([
          "name": .string(name), "arguments": .object([:]),
        ]))
      XCTAssertNotNil(rejected.1["error"].string)
    }
    let write = try await request(
      "/call",
      body: .object([
        "name": .string("tonk_apply"), "arguments": .object([:]),
      ]))
    XCTAssertNotNil(write.1["error"].string)
    XCTAssertEqual(calls, 1)
  }
}
