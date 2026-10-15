import Foundation
import HarnessCore
import XCTest

@testable import Tonk

final class ChatSessionTests: XCTestCase {
  func testNeutralInstructionsIgnoreLegacyPersonality() {
    var profile = AgentProfile()
    profile.name = "LegacyName"
    profile.soul = "Unique legacy persona"
    XCTAssertFalse(profile.instructions.contains("LegacyName"))
    XCTAssertFalse(profile.instructions.contains("Unique legacy persona"))
    XCTAssertTrue(profile.instructions.contains("neutral assistant"))
  }

  private func space(_ suffix: String) throws -> TonkSpace {
    try JSONDecoder().decode(
      TonkSpace.self,
      from: Data("{\"subject\":\"did:key:z\(suffix)\",\"name\":\"Space \(suffix)\"}".utf8))
  }

  func testSessionsStayScopedAndRestoreModelAndHistory() throws {
    var state = SavedState()
    state.provider = .local
    let firstModel = ModelConnection(provider: .local, model: "first")
    state.connections = ["local": firstModel]
    state.conversation.space = try space("ABC")
    state.conversation.draft = "Unsent first-space draft"
    state.conversation.messages = [ChatMessage(role: "user", text: "First chat")]
    state.conversation.apiHistory = [APIMessage(role: "user", text: "Private first-space context")]
    state.checkpointChat()
    let firstID = try XCTUnwrap(state.activeSessionID)
    state.activeSessionID = UUID().uuidString
    state.conversation = Conversation()
    state.conversation.space = try space("DEF")
    state.conversation.messages = [ChatMessage(role: "user", text: "Second chat")]
    state.connections = ["local": ModelConnection(provider: .local, model: "second")]
    state.checkpointChat()
    XCTAssertEqual(state.chats(in: "did:key:zABC").map(\.title), ["First chat"])
    XCTAssertEqual(state.chats(in: "did:key:zDEF").map(\.title), ["Second chat"])
    try state.restoreChat(firstID)
    XCTAssertEqual(state.conversation.draft, "Unsent first-space draft")
    XCTAssertEqual(state.connections?["local"], firstModel)
    XCTAssertEqual(state.conversation.apiHistory?.first?.text, "Private first-space context")
    XCTAssertEqual(state.conversation.space?.id, "did:key:zABC")
    XCTAssertThrowsError(try state.restoreChat("missing"))
  }

  @MainActor
  func testLegacyImportIsIdempotentAndPreservesOriginalFiles() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    var current = SavedState()
    current.conversation.space = try space("ABC")
    current.conversation.messages = [ChatMessage(role: "user", text: "Current")]
    try StateStore(directory: root).save(current)
    var archived = SavedState()
    archived.conversation.messages = [ChatMessage(role: "user", text: "Older unassigned chat")]
    let directory = root.appendingPathComponent("Conversations/" + UUID().uuidString)
    try StateStore(directory: directory).save(archived)
    let original = try Data(contentsOf: directory.appendingPathComponent("state.json"))
    let model = HarnessModel(directory: root)
    XCTAssertEqual(model.saved.sessions?.count, 2)
    XCTAssertEqual(model.saved.chats(in: nil).first?.title, "Older unassigned chat")
    let reopened = HarnessModel(directory: root)
    XCTAssertEqual(reopened.saved.sessions, model.saved.sessions)
    XCTAssertEqual(try Data(contentsOf: directory.appendingPathComponent("state.json")), original)
    model.newConversation()
    XCTAssertEqual(model.saved.conversation.space?.id, "did:key:zABC")
    XCTAssertEqual(model.saved.chats(in: "did:key:zABC").count, 2)
  }

  @MainActor
  func testCorruptArchiveDoesNotOverwriteOriginalOrAllowChatWrites() throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    try StateStore(directory: root).save(SavedState())
    let original = try Data(contentsOf: root.appendingPathComponent("state.json"))
    let archive = root.appendingPathComponent("Conversations/" + UUID().uuidString)
    try FileManager.default.createDirectory(at: archive, withIntermediateDirectories: true)
    try Data("broken".utf8).write(to: archive.appendingPathComponent("state.json"))
    let model = HarnessModel(directory: root)
    XCTAssertFalse(model.storageAvailable)
    XCTAssertNotNil(model.error)
    model.newConversation()
    model.persist()
    XCTAssertEqual(try Data(contentsOf: root.appendingPathComponent("state.json")), original)
  }

  @MainActor
  func testOpeningSpaceResumesItsLatestChatAndBlocksSwitchDuringTurn() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let model = HarnessModel(directory: root)
    try await model.configureProvider(
      .local, connection: ModelConnection(provider: .local, model: "fixture"), key: "")
    let first = try space("ABC")
    let second = try space("DEF")
    _ = await model.openSpaceChat(first)
    model.saved.conversation.messages = [ChatMessage(role: "user", text: "Keep this chat")]
    model.persist()
    let id = model.saved.activeSessionID
    _ = await model.openSpaceChat(second)
    XCTAssertEqual(model.saved.conversation.space, second)
    _ = await model.openSpaceChat(first)
    XCTAssertEqual(model.saved.activeSessionID, id)
    XCTAssertEqual(model.saved.conversation.messages.first?.text, "Keep this chat")
    model.busy = true
    let opened = await model.openSpaceChat(second)
    XCTAssertFalse(opened)
    XCTAssertEqual(model.saved.conversation.space, first)
  }
  @MainActor
  func testSpaceOpensBeforeProviderHandshakeForNewAndRestoredChats() async throws {
    for restoreChat in [false, true] {
      let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
      let release = root.appendingPathComponent("release")
      let executable = root.appendingPathComponent("codex-fixture")
      try """
      #!/usr/bin/python3
      import json, pathlib, sys, time
      release = pathlib.Path(__file__).parent / 'release'
      for line in sys.stdin:
          request = json.loads(line)
          if 'id' in request:
              while not release.exists(): time.sleep(0.01)
              print(json.dumps({'id': request['id'], 'result': {}}), flush=True)
      """.write(to: executable, atomically: true, encoding: .utf8)
      try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
      let model = HarnessModel(directory: root)
      defer {
        model.client.stop()
        try? FileManager.default.removeItem(at: root)
      }
      model.saved.codexExecutable = executable.path
      let target = try space("ABC")
      if restoreChat {
        model.attachSpace(target)
        model.saved.conversation.messages = [ChatMessage(role: "user", text: "Saved conversation")]
        model.attachSpace(try space("DEF"))
      }
      XCTAssertFalse(model.connecting)
      XCTAssertFalse(model.connected)
      let opened = expectation(description: "Space opens while provider is held unready")
      Task {
        let success = await model.enterSpace(target)
        XCTAssertTrue(success)
        opened.fulfill()
      }
      await fulfillment(of: [opened], timeout: 1)
      XCTAssertEqual(model.openedSpace, target)
      XCTAssertTrue(model.connecting)
      XCTAssertFalse(model.canSend)
      if restoreChat {
        XCTAssertEqual(model.saved.conversation.messages.first?.text, "Saved conversation")
      }
      // Release the handshake only after proving entry did not await it.
      try Data().write(to: release)
      for _ in 0..<600 {
        if !model.connecting { break }
        try await Task.sleep(for: .milliseconds(20))
      }
      XCTAssertFalse(model.connecting)
      XCTAssertTrue(model.connected, model.error ?? "Provider did not connect")
    }
  }

}
