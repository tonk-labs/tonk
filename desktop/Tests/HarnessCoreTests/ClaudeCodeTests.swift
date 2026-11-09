import XCTest

@testable import HarnessCore

final class ClaudeCodeTests: XCTestCase {
  @MainActor
  func testInstalledClaudeModelDiscoveryWithoutPrompt() async throws {
    guard ProcessInfo.processInfo.environment["TONK_TEST_CLAUDE_DISCOVERY"] == "1" else {
      throw XCTSkip("Set TONK_TEST_CLAUDE_DISCOVERY=1 to probe the installed CLI")
    }
    let client = ClaudeCodeClient()
    try await client.discover()
    let workspace = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer {
      client.stop()
      try? FileManager.default.removeItem(at: workspace)
    }
    let models = try await client.models(workspace: workspace)
    XCTAssertFalse(models.isEmpty)
    XCTAssertTrue(models.allSatisfy { !$0.id.isEmpty && !$0.name.isEmpty })
  }

  @MainActor
  func testModelDiscoveryCorrelatesResponseAndRejectsMissingCatalog() throws {
    func event(_ id: String, _ models: [JSONValue]) -> JSONValue {
      .object([
        "type": .string("control_response"),
        "response": .object([
          "request_id": .string(id), "subtype": .string("success"),
          "response": .object(["models": .array(models)]),
        ]),
      ])
    }
    let row: JSONValue = .object([
      "value": .string("sonnet"), "displayName": .string("Sonnet"),
      "description": .string("Description"),
    ])
    let models = try ClaudeCodeClient.decodeModels(
      [event("other", []), event("wanted", [row, row, .object([:])])], requestID: "wanted")
    XCTAssertEqual(models.map(\.id), ["sonnet"])
    XCTAssertEqual(models.first?.description, "Description")
    XCTAssertThrowsError(
      try ClaudeCodeClient.decodeModels([event("other", [row])], requestID: "wanted"))
    XCTAssertThrowsError(try ClaudeCodeClient.decodeModels([], requestID: "wanted"))
  }

  @MainActor
  func testLiveClaudeSubscriptionToolAndResume() async throws {
    guard ProcessInfo.processInfo.environment["TONK_TEST_CLAUDE_LIVE"] == "1" else {
      throw XCTSkip("Set TONK_TEST_CLAUDE_LIVE=1 for a live subscription smoke test")
    }
    let client = ClaudeCodeClient()
    try await client.discover()
    let workspace = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer {
      client.stop()
      try? FileManager.default.removeItem(at: workspace)
    }
    let authenticated = try await client.authenticated(workspace: workspace)
    XCTAssertTrue(authenticated)
    var calls = 0
    let bridge = LocalRuntimeBridge(agentTools: [SpaceTools.inspectionDefinition]) { _, _ in
      calls += 1
      return SpaceTools.response("The fixture word is marigold.", success: true)
    }
    let connection = try await bridge.start()
    defer { bridge.stop() }
    let config = JSONValue.object([
      "mcpServers": .object([
        "tonk": .object([
          "type": .string("http"), "url": .string(connection["url"].string! + "/mcp"),
          "headers": .object(["Authorization": .string("Bearer " + connection["token"].string!)]),
        ])
      ])
    ])
    let mcp = String(decoding: try JSONEncoder().encode(config), as: UTF8.self)
    let session = UUID().uuidString
    var text = ""
    try await client.turn(
      prompt: "Call tonk_inspect_view once and report its fixture word.",
      session: session, resume: false, model: "haiku", workspace: workspace, mcp: mcp,
      instructions: "You are testing a mock Tonk tool. Use the supplied tool when requested."
    ) { _, delta in text += delta }
    XCTAssertEqual(calls, 1)
    XCTAssertTrue(text.lowercased().contains("marigold"), text)
    text = ""
    try await client.turn(
      prompt: "What was the fixture word? Answer without calling tools.",
      session: session, resume: true, model: "haiku", workspace: workspace, mcp: mcp,
      instructions: "You are testing a mock Tonk tool. Use the supplied tool when requested."
    ) { _, delta in text += delta }
    XCTAssertEqual(calls, 1)
    XCTAssertTrue(text.lowercased().contains("marigold"), text)
  }

  @MainActor
  func testProcessStreamsPromptAndCancellationReleasesClient() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let executable = directory.appendingPathComponent("claude")
    try """
    #!/usr/bin/python3
    import sys,json,time
    if sys.argv[1:3] == ['auth','status']:
        print(json.dumps({'loggedIn':True,'authMethod':'claude.ai'}),flush=True)
    else:
        prompt=sys.stdin.read()
        if prompt == 'wait': time.sleep(30)
        print(json.dumps({'type':'assistant','message':{'id':'fixture','content':[{'type':'text','text':prompt}]}}),flush=True)
        print(json.dumps({'type':'result','subtype':'success','is_error':False}),flush=True)
    """.write(to: executable, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
    let client = ClaudeCodeClient(executable: executable)
    let authenticated = try await client.authenticated(workspace: directory)
    XCTAssertTrue(authenticated)
    var text = ""
    try await client.turn(
      prompt: "hello", session: UUID().uuidString, resume: false, model: "", workspace: directory,
      mcp: "{}", instructions: "fixture"
    ) { _, delta in text += delta }
    XCTAssertEqual(text, "hello")
    let task = Task {
      try await client.turn(
        prompt: "wait", session: UUID().uuidString, resume: false, model: "", workspace: directory,
        mcp: "{}", instructions: "fixture"
      ) { _, _ in }
    }
    try await Task.sleep(for: .milliseconds(100))
    task.cancel()
    do {
      try await task.value
      XCTFail("Cancellation was ignored")
    } catch is CancellationError {}
    let reconnected = try await client.authenticated(workspace: directory)
    XCTAssertTrue(reconnected)
  }

  func testStreamDoesNotDuplicateCompletedAssistantAndRejectsFailedResult() throws {
    var stream = ClaudeCodeStream()
    func json(_ text: String) throws -> JSONValue {
      try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    }
    XCTAssertNil(
      try stream.receive(
        json(#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"one"}}}"#)))
    let delta = try stream.receive(
      json(#"{"type":"stream_event","event":{"delta":{"type":"text_delta","text":"Hello"}}}"#))
    XCTAssertEqual(delta?.0, "one")
    XCTAssertEqual(delta?.1, "Hello")
    XCTAssertNil(
      try stream.receive(
        json(
          #"{"type":"assistant","message":{"id":"one","content":[{"type":"text","text":"Hello"}]}}"#
        )))
    XCTAssertFalse(stream.completed)
    XCTAssertThrowsError(
      try stream.receive(json(#"{"type":"result","subtype":"error_max_turns","is_error":true}"#)))
    XCTAssertFalse(stream.completed)
    _ = try stream.receive(json(#"{"type":"result","subtype":"success","is_error":false}"#))
    XCTAssertTrue(stream.completed)
  }

  @MainActor
  func testSubscriptionLaunchHasOnlyExplicitToolsAndNoAPIKeyFallback() {
    let args = ClaudeCodeClient.arguments(
      session: "session", resume: true, model: "", mcp: "{}", instructions: "Tonk")
    XCTAssertEqual(args[args.firstIndex(of: "--tools")! + 1], "")
    XCTAssertTrue(args.contains("--strict-mcp-config"))
    XCTAssertTrue(args.contains("--resume"))
    XCTAssertFalse(args.contains("--bare"))
    XCTAssertFalse(args.contains("--dangerously-skip-permissions"))
    XCTAssertEqual(args[args.firstIndex(of: "--setting-sources")! + 1], "")
    let env = ClaudeCodeClient.environment([
      "ANTHROPIC_API_KEY": "secret", "CLAUDE_CODE_OAUTH_TOKEN": "secret",
      "CLAUDE_CODE_USE_BEDROCK": "1", "PATH": "/bin",
    ])
    XCTAssertNil(env["ANTHROPIC_API_KEY"])
    XCTAssertNil(env["CLAUDE_CODE_OAUTH_TOKEN"])
    XCTAssertNil(env["CLAUDE_CODE_USE_BEDROCK"])
    XCTAssertEqual(env["PATH"], "/bin")
    XCTAssertFalse(ModelProvider.claude.requiresKey)
    XCTAssertThrowsError(try ModelConnection(provider: .claude).endpoint())
  }

  @MainActor
  func testMCPRoutesOnlyAdvertisedToolsAndConvertsToolResults() async throws {
    var calls = 0
    let bridge = LocalRuntimeBridge(agentTools: [SpaceTools.inspectionDefinition]) { name, _ in
      calls += 1
      XCTAssertEqual(name, "tonk_inspect_view")
      return SpaceTools.response("Rendered view", success: true)
    }
    let config = try await bridge.start()
    defer { bridge.stop() }
    func request(_ method: String, name: String = "", authorized: Bool = true) async throws -> (
      Int, JSONValue
    ) {
      var request = URLRequest(url: URL(string: config["url"].string! + "/mcp")!)
      request.httpMethod = "POST"
      request.setValue("application/json", forHTTPHeaderField: "Content-Type")
      if authorized {
        request.setValue("Bearer " + config["token"].string!, forHTTPHeaderField: "Authorization")
      }
      request.httpBody = try JSONEncoder().encode(
        JSONValue.object([
          "jsonrpc": .string("2.0"), "id": .number(1), "method": .string(method),
          "params": .object(["name": .string(name), "uri": .string(name), "arguments": .object([:])]),
        ]))
      let (data, response) = try await URLSession.shared.data(for: request)
      return (
        (response as! HTTPURLResponse).statusCode,
        try JSONDecoder().decode(JSONValue.self, from: data)
      )
    }
    let unauthorized = try await request("tools/list", authorized: false)
    XCTAssertEqual(unauthorized.0, 403)
    let initialized = try await request("initialize")
    XCTAssertEqual(initialized.1["result"]["protocolVersion"].string, "2025-03-26")
    XCTAssertEqual(initialized.1["result"]["instructions"].string, TonkAgentContext.instructions)
    let resources = try await request("resources/list")
    XCTAssertEqual(resources.1["result"]["resources"], .array(TonkAgentContext.resources))
    let context = try await request("resources/read", name: "tonk://guide/context")
    XCTAssertEqual(context.1["result"]["contents"].array.first?["text"].string, TonkAgentContext.instructions)
    let tools = try await request("tools/list")
    XCTAssertEqual(tools.1["result"]["tools"].array.count, 1)
    let rejected = try await request("tools/call", name: "Bash")
    XCTAssertEqual(rejected.1["error"]["code"], .number(-32602))
    let called = try await request("tools/call", name: "tonk_inspect_view")
    XCTAssertEqual(called.1["result"]["content"].array.first?["type"].string, "text")
    XCTAssertEqual(called.1["result"]["content"].array.first?["text"].string, "Rendered view")
    XCTAssertEqual(called.1["result"]["isError"].bool, false)
    XCTAssertEqual(calls, 1)
  }
}
