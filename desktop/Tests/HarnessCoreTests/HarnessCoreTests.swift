import Foundation
import XCTest

@testable import HarnessCore

final class HarnessCoreTests: XCTestCase {
  func testFramingAcrossChunksAndUnicodeBoundaries() throws {
    let data = Data("{\"text\":\"café\"}\n\n{\"id\":2}\n".utf8)
    var framer = JSONLines()
    var output: [JSONValue] = []
    for byte in data { output += try framer.append(Data([byte])) }
    XCTAssertEqual(output, [.object(["text": .string("café")]), .object(["id": .number(2)])])
  }

  func testFramingRejectsMalformedAndOversizeMessages() throws {
    var malformed = JSONLines()
    XCTAssertThrowsError(try malformed.append(Data("not json\n".utf8)))
    var large = JSONLines(maximumBytes: 4)
    XCTAssertThrowsError(try large.append(Data("12345".utf8)))
    var complete = JSONLines(maximumBytes: 4)
    XCTAssertThrowsError(try complete.append(Data("12345\n".utf8)))
  }

  func testFinalMessageReplacesDeltasWithoutDuplicatingAndPreservesOrder() {
    var conversation = Conversation()
    conversation.messages.append(ChatMessage(id: "user", role: "user", text: "Hello"))
    conversation.appendDelta(itemID: "first", text: "Hel")
    conversation.appendDelta(itemID: "first", text: "lo")
    conversation.appendDelta(itemID: "second", text: "Another thought")
    conversation.completeMessage(itemID: "first", text: "Hello.")
    XCTAssertEqual(conversation.messages.map(\.id), ["user", "first", "second"])
    XCTAssertEqual(conversation.messages[1].text, "Hello.")
    conversation.completeMessage(itemID: "no-deltas", text: "Complete response")
    XCTAssertEqual(conversation.messages.last?.text, "Complete response")
  }

  func testStateRoundTripAndCorruptStateIsNotSilentlyReplaced() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let store = StateStore(directory: directory)
    XCTAssertEqual(try store.load(), SavedState())
    var state = SavedState()
    state.profile.name = "Finch"
    state.conversation.threadID = "thread-123"
    state.conversation.lastTurnStatus = "interrupted"
    state.conversation.messages.append(ChatMessage(role: "user", text: "Keep this"))
    try store.save(state)
    XCTAssertEqual(try store.load(), state)
    let file = directory.appendingPathComponent("state.json")
    let attributes = try FileManager.default.attributesOfItem(atPath: file.path)
    XCTAssertEqual((attributes[.posixPermissions] as? NSNumber)?.intValue, 0o600)
    try Data("broken".utf8).write(to: file)
    XCTAssertThrowsError(try store.load())
    XCTAssertEqual(try String(contentsOf: file), "broken")
  }

  func testRuntimeNavigationBoundary() {
    XCTAssertTrue(RuntimeLocation.isEmbedded(URL(string: "https://tonk.network/a/path")!))
    for address in [
      "https://tonk.network.evil.example", "http://tonk.network", "https://tonk.network:444",
      "https://user@tonk.network", "file:///tmp/test.html", "javascript:alert(1)",
    ] {
      XCTAssertFalse(RuntimeLocation.isEmbedded(URL(string: address)!))
    }
    XCTAssertFalse(RuntimeLocation.isExternal(URL(string: "file:///tmp/test.html")!))
    XCTAssertFalse(RuntimeLocation.isExternal(URL(string: "javascript:alert(1)")!))
  }

  @MainActor
  func testRealProcessTransportHandshakeResponsesTimeoutAndUnsupportedRequest() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let executable = directory.appendingPathComponent("fake-agent")
    try """
    #!/usr/bin/python3
    import json, sys
    initialized = False
    for line in sys.stdin:
        request = json.loads(line)
        method = request.get('method')
        if method == 'initialized':
            initialized = True
            continue
        if method == 'test/timeout':
            continue
        if method == 'test/tool':
            print(json.dumps({'id':'native-tool','method':'item/tool/call','params':{'tool':'tonk_space_info','arguments':{}}}), flush=True)
            print(json.dumps({'id':request['id'],'result':{}}), flush=True)
            continue
        if request.get('id') == 'native-tool':
            print(json.dumps({'method':'test/tool-result','params':request['result']}), flush=True)
            continue
        if method == 'test/unsupported':
            print(json.dumps({'id':'server-request','method':'unknown/tool','params':{}}), flush=True)
            continue
        if request.get('id') == 'server-request':
            print(json.dumps({'method':'test/denied','params':request}), flush=True)
            continue
        result = {'ok': True} if method == 'initialize' else {'initialized':initialized, 'echo':request.get('params')}
        print(json.dumps({'id': request['id'], 'result': result}), flush=True)
    """.write(to: executable, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
    let client = AppServerClient()
    defer { client.stop() }
    try await client.start(
      executable: executable, home: directory.appendingPathComponent("home"), workspace: directory)
    let response = try await client.request("test/echo", params: .string("hello"))
    XCTAssertEqual(response["initialized"], .bool(true))
    XCTAssertEqual(response["echo"], .string("hello"))
    do {
      _ = try await client.request("test/timeout", timeout: 0.05)
      XCTFail("Expected timeout")
    } catch { XCTAssertTrue(error.localizedDescription.contains("timed out")) }
    let denied = expectation(description: "Unsupported server request is rejected")
    client.onNotification = { method, params in
      if method == "test/denied" {
        XCTAssertEqual(params["error"]["code"].int, -32601)
        denied.fulfill()
      }
    }
    Task { _ = try? await client.request("test/unsupported", timeout: 0.2) }
    await fulfillment(of: [denied], timeout: 2)
    let toolResult = expectation(description: "Native tool response crosses the process protocol")
    client.onToolCall = { params in
      XCTAssertEqual(params["tool"].string, "tonk_space_info")
      return SpaceTools.response("worker result", success: true)
    }
    client.onNotification = { method, params in
      if method == "test/tool-result" {
        XCTAssertEqual(params["success"].bool, true)
        XCTAssertEqual(params["contentItems"].array.first?["text"].string, "worker result")
        toolResult.fulfill()
      }
    }
    _ = try await client.request("test/tool")
    await fulfillment(of: [toolResult], timeout: 2)
    client.stop()
    do {
      _ = try await client.request("test/echo")
      XCTFail("Expected disconnection")
    } catch { XCTAssertTrue(error.localizedDescription.contains("disconnected")) }
  }
}
