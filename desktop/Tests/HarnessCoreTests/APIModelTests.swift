import Foundation
import XCTest

@testable import HarnessCore

final class APIModelTests: XCTestCase {
  func testGeminiToolSignatureSurvivesStreamStorageAndReplay() throws {
    var decoder = APIStreamDecoder(anthropic: false)
    _ = try decoder.accept(
      #"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"tonk_query","arguments":"{}"},"extra_content":{"google":{"thought_signature":"opaque-signature"}}}]}}]}"#
    )
    _ = try decoder.accept(#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#)
    _ = try decoder.accept("[DONE]")
    let reply = try JSONDecoder().decode(
      APIMessage.self, from: JSONEncoder().encode(decoder.result()))
    let request = try APIModelWire.request(
      connection: ModelConnection(provider: .gemini, model: "fixture"), key: "fixture-key",
      instructions: "", history: [reply], tools: [])
    let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
    XCTAssertEqual(
      body["messages"].array[1]["tool_calls"].array[0]["extra_content"]["google"][
        "thought_signature"
      ].string, "opaque-signature")
    XCTAssertEqual(
      request.url?.absoluteString,
      "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions")
  }

  func testEndpointsAndMigration() throws {
    for provider in [
      ModelProvider.openAI, .anthropic, .openRouter, .gemini, .groq, .mistral, .ollama, .grok,
      .local,
    ] {
      let endpoint = try ModelConnection(provider: provider, model: "test-model").endpoint()
      XCTAssertEqual(
        endpoint.lastPathComponent, provider == .anthropic ? "messages" : "completions")
    }
    for url in [
      "http://example.com/v1", "https://user:secret@example.com/v1",
      "https://example.com/?key=secret", "file:///tmp/model",
    ] {
      XCTAssertThrowsError(
        try ModelConnection(provider: .compatible, baseURL: url, model: "test").endpoint())
    }
    XCTAssertThrowsError(
      try ModelConnection(provider: .local, baseURL: "https://example.com/v1", model: "test")
        .endpoint())
    XCTAssertThrowsError(try ModelConnection(provider: .anthropic, model: "").endpoint())
    let old = #"{"profile":{"name":"Robin","soul":"Hello"},"conversation":{"messages":[]}}"#
    let saved = try JSONDecoder().decode(SavedState.self, from: Data(old.utf8))
    XCTAssertNil(saved.provider)
    XCTAssertNil(saved.conversation.apiHistory)
  }

  func testProviderWireFormatsAlwaysIncludeTools() throws {
    let call = APIToolCall(id: "call-1", name: "tonk_guide", arguments: .object([:]))
    let history = [
      APIMessage(role: "user", text: "Inspect"), APIMessage(role: "assistant", calls: [call]),
      APIMessage(role: "tool", text: "result", toolID: call.id),
    ]
    for provider in [
      ModelProvider.openAI, .anthropic, .openRouter, .gemini, .groq, .mistral, .ollama, .grok,
      .local, .compatible,
    ] {
      let config = ModelConnection(
        provider: provider, baseURL: provider == .compatible ? "https://example.com/v1" : nil,
        model: "test")
      let request = try APIModelWire.request(
        connection: config, key: "fixture-key", instructions: "Soul", history: history,
        tools: SpaceTools.agentDefinitions(includeCLI: false))
      let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
      XCTAssertEqual(body["model"].string, "test")
      XCTAssertTrue(body["stream"].bool == true)
      XCTAssertFalse(body["tools"].array.isEmpty)
      if provider == .anthropic {
        XCTAssertEqual(request.value(forHTTPHeaderField: "x-api-key"), "fixture-key")
        XCTAssertNil(request.value(forHTTPHeaderField: "Authorization"))
        XCTAssertEqual(body["system"].string, "Soul")
        XCTAssertEqual(body["messages"].array[2]["content"].array[0]["tool_use_id"].string, call.id)
      } else {
        XCTAssertEqual(request.value(forHTTPHeaderField: "Authorization"), "Bearer fixture-key")
        XCTAssertEqual(body["messages"].array[0]["content"].string, "Soul")
        XCTAssertEqual(body["messages"].array[3]["tool_call_id"].string, call.id)
      }
      if provider == .openRouter {
        XCTAssertEqual(body["provider"]["require_parameters"].bool, true)
      } else {
        XCTAssertEqual(body["provider"], .null)
      }
    }
  }

  func testLegacyTextOnlyConfigurationNowIncludesTools() throws {
    let legacy =
      #"{"provider":"local","baseURL":"http://localhost:1234/v1","model":"fixture","toolsEnabled":false}"#
    let connection = try JSONDecoder().decode(ModelConnection.self, from: Data(legacy.utf8))
    let request = try APIModelWire.request(
      connection: connection, key: "", instructions: "", history: [],
      tools: SpaceTools.agentDefinitions(includeCLI: false))
    let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
    XCTAssertFalse(body["tools"].array.isEmpty)
    let saved = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(connection))
    XCTAssertEqual(saved["toolsEnabled"], .null)
  }

  func testFragmentedOpenAIToolsAndTruncation() throws {
    var decoder = APIStreamDecoder(anthropic: false)
    XCTAssertEqual(
      try decoder.accept(#"{"choices":[{"delta":{"content":"Checking "}}]}"#), "Checking ")
    try decoder.acceptIgnoringText(
      #"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","function":{"name":"tonk_query","arguments":"{\"query\":"}}]}}]}"#
    )
    XCTAssertThrowsError(try decoder.result())
    try decoder.acceptIgnoringText(
      #"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"hello\"}"}}]},"finish_reason":"tool_calls"}]}"#
    )
    try decoder.acceptIgnoringText("[DONE]")
    XCTAssertEqual(
      try decoder.result().calls.first?.arguments, .object(["query": .string("hello")]))
    var limited = APIStreamDecoder(anthropic: false)
    try limited.acceptIgnoringText(
      #"{"choices":[{"delta":{"content":"partial"},"finish_reason":"length"}]}"#)
    try limited.acceptIgnoringText("[DONE]")
    XCTAssertThrowsError(try limited.result())
    XCTAssertThrowsError(try limited.accept(#"{"error":{"message":"failure"}}"#))
  }

  func testAnthropicStreamAndMalformedArguments() throws {
    var decoder = APIStreamDecoder(anthropic: true)
    for event in [
      #"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"tool-1","name":"tonk_guide","input":{}}}"#,
      #"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
      #"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
      #"{"type":"message_stop"}"#,
    ] { try decoder.acceptIgnoringText(event) }
    XCTAssertEqual(try decoder.result().calls.first?.name, "tonk_guide")
    var invalid = APIStreamDecoder(anthropic: false)
    try invalid.acceptIgnoringText(
      #"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"x","function":{"name":"write","arguments":"{"}}]},"finish_reason":"tool_calls"}]}"#
    )
    try invalid.acceptIgnoringText("[DONE]")
    XCTAssertThrowsError(try invalid.result())
  }

  func testInterruptedToolRepairKeepsKnownResults() {
    let calls = ["one", "two"].map { APIToolCall(id: $0, name: "write", arguments: .object([:])) }
    let history = [
      APIMessage(role: "assistant", calls: calls),
      APIMessage(role: "tool", text: "written", toolID: "one"),
    ]
    let repaired = APIMessage.repairingInterruptedTools(history)
    XCTAssertEqual(repaired.prefix(2), history[...])
    XCTAssertEqual(repaired.last?.toolID, "two")
    XCTAssertTrue(repaired.last?.isError == true)
    XCTAssertEqual(APIMessage.repairingInterruptedTools(repaired), repaired)
  }

  @MainActor
  func testToolLoopReturnsResultsAndDoesNotRetryFailures() async throws {
    var requests = 0
    var executed = 0
    var saved: [APIMessage] = []
    let client = APIModelClient { request, _, delta in
      requests += 1
      if requests == 1 {
        return APIMessage(
          role: "assistant",
          calls: [APIToolCall(id: "one", name: "tonk_guide", arguments: .object([:]))])
      }
      let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
      XCTAssertEqual(body["messages"].array.last?["tool_call_id"].string, "one")
      delta("Done")
      return APIMessage(role: "assistant", text: "Done")
    }
    try await client.run(
      connection: ModelConnection(provider: .local, model: "fixture"), key: "",
      instructions: "Test", history: [], tools: SpaceTools.agentDefinitions(includeCLI: false),
      onHistory: { saved = $0 }, onText: { _, _ in },
      execute: { _ in
        executed += 1
        return SpaceTools.response("Space information", success: true)
      })
    XCTAssertEqual(requests, 2)
    XCTAssertEqual(executed, 1)
    XCTAssertEqual(saved.map(\.role), ["assistant", "tool", "assistant"])
    var failures = 0
    let failing = APIModelClient { _, _, _ in
      failures += 1
      throw HarnessError.message("failed")
    }
    do {
      try await failing.run(
        connection: ModelConnection(provider: .local, model: "fixture"), key: "", instructions: "",
        history: [], tools: [], onHistory: { _ in }, onText: { _, _ in },
        execute: { _ in
          XCTFail("Must not execute")
          return .null
        })
      XCTFail("Expected failure")
    } catch { XCTAssertEqual(failures, 1) }
  }
  @MainActor
  func testCancellationSavesCompletedToolAndDoesNotRunNextTool() async throws {
    var saved: [APIMessage] = []
    var executionCount = 0
    var task: Task<Void, Error>!
    let client = APIModelClient { _, _, _ in
      APIMessage(
        role: "assistant",
        calls: ["one", "two"].map {
          APIToolCall(id: $0, name: "tonk_guide", arguments: .object([:]))
        })
    }
    task = Task {
      try await client.run(
        connection: ModelConnection(provider: .local, model: "fixture"), key: "", instructions: "",
        history: [], tools: SpaceTools.agentDefinitions(includeCLI: false),
        onHistory: { saved = $0 }, onText: { _, _ in },
        execute: { _ in
          executionCount += 1
          task.cancel()
          return SpaceTools.response("Completed before cancellation", success: true)
        })
    }
    do {
      try await task.value
      XCTFail("Expected cancellation")
    } catch is CancellationError {} catch { throw error }
    XCTAssertEqual(executionCount, 1)
    XCTAssertEqual(saved.last?.toolID, "one")
    XCTAssertTrue(saved.last?.text.contains("Completed before cancellation") == true)
    XCTAssertEqual(APIMessage.repairingInterruptedTools(saved).last?.toolID, "two")
  }

  @MainActor
  func testUnknownToolsAreDeclinedAndFailedJournalPreventsExecution() async throws {
    var requests = 0
    let client = APIModelClient { request, _, _ in
      requests += 1
      if requests == 1 {
        return APIMessage(
          role: "assistant", calls: [APIToolCall(id: "bad", name: "shell", arguments: .object([:]))]
        )
      }
      let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
      XCTAssertTrue(body["messages"].array.last?["content"].string?.contains("unavailable") == true)
      return APIMessage(role: "assistant", text: "Tool unavailable")
    }
    try await client.run(
      connection: ModelConnection(provider: .local, model: "fixture"), key: "", instructions: "",
      history: [], tools: [], onHistory: { _ in }, onText: { _, _ in },
      execute: { _ in
        XCTFail("Unknown tool executed")
        return .null
      })
    XCTAssertEqual(requests, 2)
    let write = APIModelClient { _, _, _ in
      APIMessage(
        role: "assistant",
        calls: [APIToolCall(id: "one", name: "tonk_guide", arguments: .object([:]))])
    }
    do {
      try await write.run(
        connection: ModelConnection(provider: .local, model: "fixture"), key: "", instructions: "",
        history: [], tools: SpaceTools.agentDefinitions(includeCLI: false),
        onHistory: { _ in throw HarnessError.message("Disk full") }, onText: { _, _ in },
        execute: { _ in
          XCTFail("Executed without durable intent")
          return .null
        })
      XCTFail("Expected journal failure")
    } catch { XCTAssertEqual(error.localizedDescription, "Disk full") }
  }

}

extension APIStreamDecoder {
  fileprivate mutating func acceptIgnoringText(_ data: String) throws { _ = try accept(data) }
}
