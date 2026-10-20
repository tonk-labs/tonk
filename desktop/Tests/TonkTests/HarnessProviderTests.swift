import Foundation
import HarnessCore
import XCTest

@testable import Tonk

final class HarnessProviderTests: XCTestCase {
  @MainActor
  func testAttachedSpaceReadGuidanceReachesProviderWithoutPreparingCLI() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    var requests = 0
    let question = "what issues are active and assigned to jack?"
    let client = APIModelClient { request, _, delta in
      requests += 1
      let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
      let instructions = try XCTUnwrap(body["messages"].array.first?["content"].string)
      XCTAssertTrue(instructions.contains(TonkAgentContext.instructions))
      XCTAssertTrue(instructions.contains("Attached Tonk space: did:key:zABC"))
      XCTAssertTrue(instructions.contains("narrow sidebar"))
      XCTAssertEqual(body["messages"].array.last?["content"].string, question)
      delta("Fixture response.")
      return APIMessage(role: "assistant", text: "Fixture response.")
    }
    let model = HarnessModel(directory: directory, apiClient: client)
    try await model.configureProvider(
      .local, connection: ModelConnection(provider: .local, model: "fixture"), key: "")
    model.attachSpace(
      try JSONDecoder().decode(
        TonkSpace.self, from: Data(#"{"subject":"did:key:zABC","name":"Test space"}"#.utf8)))
    await model.send(question)
    await model.apiTask?.value
    XCTAssertEqual(requests, 1)
    XCTAssertNil(model.error)
    XCTAssertTrue(model.cliAdapters.isEmpty)
    XCTAssertTrue(model.saved.conversation.messages.allSatisfy { ($0.toolActivity ?? []).isEmpty })
  }

  @MainActor
  func testLateLoginCompletionDoesNotShowCancellationError() {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let model = HarnessModel(directory: directory)
    model.loginPending = false
    model.client.onNotification?(
      "account/login/completed",
      .object([
        "loginId": .string("cancelled-attempt"),
        "success": .bool(false),
        "error": .string("Login server error: Login was not completed"),
      ]))
    XCTAssertNil(model.error)
    XCTAssertFalse(model.loginPending)

    // A late completion must not disturb a newer pending login either.
    model.loginPending = true
    model.client.onNotification?(
      "account/login/completed",
      .object([
        "loginId": .string("cancelled-attempt"),
        "success": .bool(false),
        "error": .string("Login was not completed"),
      ]))
    XCTAssertNil(model.error)
    XCTAssertTrue(model.loginPending)
  }

  @MainActor
  func testExecutablePickerRequiresDiscoveryFailureRatherThanConnectionFailure() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let model = HarnessModel(directory: directory)
    defer { model.client.stop() }
    XCTAssertFalse(model.codexDiscoveryFailed)

    model.saved.codexExecutable = directory.appendingPathComponent("missing-codex").path
    await model.connect()
    XCTAssertTrue(model.codexDiscoveryFailed)
    XCTAssertFalse(model.connected)

    // This executable is found, but exits without completing the app-server handshake.
    model.saved.codexExecutable = "/usr/bin/false"
    await model.connect()
    XCTAssertFalse(model.codexDiscoveryFailed)
    XCTAssertFalse(model.connected)
    XCTAssertNotNil(model.error)
  }

  @MainActor
  func testCodexSelectionPersistsAndReconnectsWithoutArchivingConversation() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let executable = directory.appendingPathComponent("selected codex")
    try """
    #!/usr/bin/python3
    import json, sys
    for line in sys.stdin:
        request = json.loads(line)
        if 'id' in request:
            print(json.dumps({'id':request['id'], 'result':{}}), flush=True)
    """.write(to: executable, atomically: true, encoding: .utf8)
    try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
    let model = HarnessModel(directory: directory)
    defer { model.client.stop() }
    model.saved.conversation.messages = [ChatMessage(role: "user", text: "Keep this conversation")]
    model.saved.conversation.threadID = "existing-thread"
    // An existing connection must also reconnect when only the executable changes.
    model.connected = true
    let conversation = model.saved.conversation
    try await model.configureProvider(
      .chatGPT, connection: model.connection, key: "", codexExecutable: executable.path)
    XCTAssertTrue(model.connected)
    XCTAssertNil(model.error)
    XCTAssertFalse(model.codexDiscoveryFailed)
    _ = try await model.client.request("test/connected-to-selection")
    XCTAssertEqual(model.saved.conversation, conversation)
    let reopened = HarnessModel(directory: directory)
    XCTAssertEqual(reopened.saved.codexExecutable, executable.path)

    do {
      try await model.configureProvider(
        .chatGPT, connection: model.connection, key: "", codexExecutable: "/missing-codex")
      XCTFail("Accepted stale executable")
    } catch {}
    XCTAssertEqual(model.saved.codexExecutable, executable.path)
    XCTAssertTrue(model.connected)

    try await model.configureProvider(
      .disabled, connection: ModelConnection(provider: .disabled), key: "")
    XCTAssertEqual(model.saved.codexExecutable, executable.path)
  }

  @MainActor
  func testDisabledProviderPreventsRequestsAndPreservesConfiguration() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let client = APIModelClient { _, _, _ in
      XCTFail("Disabled chat must not issue a request")
      return APIMessage(role: "assistant", text: "unexpected")
    }
    let model = HarnessModel(directory: directory, apiClient: client)
    let local = ModelConnection(provider: .ollama, model: "test")
    try await model.configureProvider(.ollama, connection: local, key: "")
    XCTAssertTrue(model.canSend)
    try await model.configureProvider(
      .disabled, connection: ModelConnection(provider: .disabled), key: "")
    XCTAssertFalse(model.canSend)
    await model.send("Do not send this")
    XCTAssertTrue(model.saved.conversation.messages.isEmpty)
    XCTAssertEqual(model.saved.connections?["ollama"], local)
    let reopened = HarnessModel(directory: directory)
    await reopened.connect()
    XCTAssertEqual(reopened.provider, .disabled)
    XCTAssertFalse(reopened.canSend)
  }

  @MainActor
  func testLocalAgentProposalPersistsAndSwitchArchivesWithoutForwardingHistory() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "provider-test-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    var requests = 0
    let client = APIModelClient { request, _, delta in
      requests += 1
      let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
      if requests == 1 {
        return APIMessage(
          role: "assistant",
          calls: [
            APIToolCall(
              id: "proposal", name: "tonk_propose_space",
              arguments: .object([
                "name": .string("Local model test"), "reason": .string("Keep a shared checklist"),
              ]))
          ])
      }
      XCTAssertEqual(body["messages"].array.last?["role"].string, "tool")
      XCTAssertTrue(body["messages"].array.last?["content"].string?.contains("true") == true)
      delta("Your proposal is ready.")
      return APIMessage(role: "assistant", text: "Your proposal is ready.")
    }
    let model = HarnessModel(directory: directory, apiClient: client)
    let connection = ModelConnection(provider: .local, model: "fixture-model")
    try await model.configureProvider(.local, connection: connection, key: "")
    XCTAssertTrue(model.canSend)
    await model.send("Make a checklist")
    await model.apiTask?.value
    XCTAssertNil(model.error)
    XCTAssertFalse(model.busy)
    XCTAssertEqual(requests, 2)
    XCTAssertEqual(model.saved.conversation.spaceProposal?.name, "Local model test")
    XCTAssertEqual(
      model.saved.conversation.messages.map(\.text),
      ["Make a checklist", "Your proposal is ready."])
    let reopened = HarnessModel(directory: directory)
    XCTAssertEqual(reopened.saved, model.saved)
    XCTAssertEqual(
      reopened.saved.conversation.apiHistory?.map(\.role),
      ["user", "assistant", "tool", "assistant"])
    var changed = connection
    changed.model = "another-fixture"
    try await model.configureProvider(.local, connection: changed, key: "")
    XCTAssertTrue(model.saved.conversation.messages.isEmpty)
    XCTAssertNil(model.saved.conversation.apiHistory)
    XCTAssertNil(model.saved.conversation.spaceProposal)
    let original = try XCTUnwrap(
      model.saved.sessions?.first { $0.id == reopened.saved.activeSessionID })
    XCTAssertEqual(original.conversation, reopened.saved.conversation)
    XCTAssertEqual(original.connection, connection)

  }

  @MainActor
  func testProviderChangeIsRejectedWhileBusyAndHiddenContinuationStaysHidden() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
      "provider-test-" + UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let client = APIModelClient { request, _, delta in
      let body = try JSONDecoder().decode(JSONValue.self, from: request.httpBody!)
      XCTAssertEqual(body["messages"].array.last?["content"].string, "Continue in the new space")
      delta("Continuing.")
      return APIMessage(role: "assistant", text: "Continuing.")
    }
    let model = HarnessModel(directory: directory, apiClient: client)
    let connection = ModelConnection(provider: .local, model: "fixture-model")
    try await model.configureProvider(.local, connection: connection, key: "")
    model.busy = true
    do {
      try await model.configureProvider(
        .chatGPT, connection: ModelConnection(provider: .chatGPT), key: "")
      XCTFail("Changed while busy")
    } catch {}
    XCTAssertEqual(model.provider, .local)
    model.busy = false
    await model.send("Continue in the new space", showsUserMessage: false)
    await model.apiTask?.value
    XCTAssertEqual(model.saved.conversation.messages.map(\.role), ["assistant"])
    XCTAssertEqual(model.saved.conversation.apiHistory?.first?.text, "Continue in the new space")
  }

  @MainActor
  func testModelChoiceSeedsTheAccountThenFollowsAnotherDesktop() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let model = HarnessModel(directory: directory)
    try await model.configureProvider(
      .local, connection: ModelConnection(provider: .local, model: "fixture"), key: "")
    let account = ModelChoiceAccountStub()
    let local = ModelChoice(provider: .local, model: "fixture")

    await model.syncModelChoice(with: account)
    XCTAssertEqual(account.written, [local])
    XCTAssertEqual(model.saved.syncedModelChoice, local)

    account.remote = .known(ModelChoice(provider: .ollama, model: "llama3"))
    await model.syncModelChoice(with: account)
    XCTAssertEqual(model.provider, .ollama)
    XCTAssertEqual(model.connection.model, "llama3")
    XCTAssertEqual(account.written, [local])
    XCTAssertEqual(try StateStore(directory: directory).load().syncedModelChoice?.provider, .ollama)

    // A choice this Mac has no key for is left for later; nothing is overwritten.
    account.remote = .known(ModelChoice(provider: .anthropic, model: "fixture-model"))
    await model.syncModelChoice(with: account)
    XCTAssertEqual(model.provider, .ollama)
    XCTAssertEqual(model.saved.syncedModelChoice?.provider, .ollama)
    XCTAssertEqual(account.written, [local])
  }
}

@MainActor
private final class ModelChoiceAccountStub: ModelChoiceAccount {
  var remote: ModelChoice.RemoteValue?
  var written: [ModelChoice] = []
  func readModelChoice() async throws -> ModelChoice.RemoteValue? { remote }
  func writeModelChoice(_ choice: ModelChoice) async throws {
    written.append(choice)
    remote = .known(choice)
  }
}
