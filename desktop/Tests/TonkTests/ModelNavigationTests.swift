import Foundation
import HarnessCore
import XCTest

@testable import Tonk

final class ModelNavigationTests: XCTestCase {
  @MainActor
  func testModelLabelUsesConfiguredAPIModelAndResolvedSubscriptionModel() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let model = HarnessModel(directory: directory)
    model.saved.provider = .openAI
    model.saved.connections = ["openAI": ModelConnection(provider: .openAI, model: "api-test")]
    XCTAssertEqual(model.modelLabel, "api-test")
    model.saved.provider = .chatGPT
    model.saved.connections?["chatGPT"] = ModelConnection(
      provider: .chatGPT, model: "subscription-test")
    XCTAssertEqual(model.modelLabel, "subscription-test")
    model.saved.conversation.resolvedModel = "server-resolved-test"
    XCTAssertEqual(model.modelLabel, "server-resolved-test")
  }

  @MainActor
  func testSavedModelsRetainMultipleModelsFromSameProviderAcrossReopening() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let model = HarnessModel(directory: directory)
    let first = ModelConnection(provider: .local, model: "first")
    let second = ModelConnection(provider: .local, model: "second")
    try await model.configureProvider(.local, connection: first, key: "")
    try await model.configureProvider(.local, connection: second, key: "")
    try await model.configureProvider(.local, connection: first, key: "")
    XCTAssertEqual(model.configuredModels.filter { $0.provider == .local }.count, 2)
    XCTAssertEqual(model.modelLabel, "first")
    let reopened = HarnessModel(directory: directory)
    XCTAssertEqual(reopened.configuredModels, model.configuredModels)
  }

  @MainActor
  func testSpaceOpensWithChatHiddenAndNewChatStaysInThatSpace() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: directory) }
    let model = HarnessModel(directory: directory)
    try await model.configureProvider(
      .local, connection: ModelConnection(provider: .local, model: "test"), key: "")
    let space = try JSONDecoder().decode(
      TonkSpace.self, from: Data(#"{"subject":"did:key:zABC","name":"Test"}"#.utf8))
    let opened = await model.enterSpace(space)
    XCTAssertTrue(opened)
    XCTAssertEqual(model.openedSpace, space)
    XCTAssertFalse(model.showingChat)
    let old = model.saved.activeSessionID
    model.startSpaceChat()
    XCTAssertTrue(model.showingChat)
    XCTAssertNotEqual(model.saved.activeSessionID, old)
    XCTAssertEqual(model.saved.conversation.space, space)
    model.saved.conversation.messages = [ChatMessage(role: "user", text: "Keep this")]
    model.persist()
    let chatID = model.saved.activeSessionID
    model.openedSpace = nil
    _ = await model.enterSpace(space)
    XCTAssertFalse(model.showingChat)
    XCTAssertEqual(model.saved.activeSessionID, chatID)
    XCTAssertEqual(
      model.saved.chats(in: space.id).filter { !$0.conversation.messages.isEmpty }.count, 1)
  }
}
