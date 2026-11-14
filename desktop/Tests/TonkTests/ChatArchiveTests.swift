import Foundation
import HarnessCore
import XCTest

@testable import Tonk

@MainActor
final class ChatArchiveTests: XCTestCase {
  private var root: URL!

  override func setUp() async throws {
    root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    var initial = SavedState()
    // No model process starts for a disabled provider.
    initial.provider = .disabled
    try StateStore(directory: root).save(initial)
  }

  override func tearDown() async throws {
    try? FileManager.default.removeItem(at: root)
  }

  private func space(_ suffix: String) throws -> TonkSpace {
    try JSONDecoder().decode(
      TonkSpace.self,
      from: Data("{\"subject\":\"did:key:z\(suffix)\",\"name\":\"Space \(suffix)\"}".utf8))
  }

  private func window(_ archive: ChatArchive) -> HarnessModel {
    HarnessModel(directory: root, store: archive.handle(claimsChats: true))
  }

  private func say(_ text: String, in model: HarnessModel) {
    model.saved.conversation.messages.append(ChatMessage(role: "user", text: text))
    model.persist()
  }

  func testTwoWindowsOnOneSpaceKeepSeparateChats() async throws {
    let archive = ChatArchive(directory: root)
    let first = window(archive)
    let second = window(archive)
    let shared = try space("ABC")
    let firstOpened = await first.openSpaceChat(shared)
    XCTAssertTrue(firstOpened)
    say("From the first window", in: first)
    let secondOpened = await second.openSpaceChat(shared)
    XCTAssertTrue(secondOpened)
    XCTAssertNotEqual(
      first.saved.activeSessionID, second.saved.activeSessionID,
      "A window never takes a chat another window has open")
    say("From the second window", in: second)
    say("More from the first window", in: first)

    let disk = try StateStore(directory: root).load()
    let chats = disk.chats(in: shared.id)
    XCTAssertEqual(
      Set(chats.map(\.title)), ["From the first window", "From the second window"])
    let firstChat = try XCTUnwrap(chats.first { $0.id == first.saved.activeSessionID })
    XCTAssertEqual(firstChat.conversation.messages.count, 2, "Neither window undoes the other")
  }

  func testOpenChatIsRefusedInAnotherWindowUntilItCloses() async throws {
    let archive = ChatArchive(directory: root)
    let first = window(archive)
    let second = window(archive)
    let shared = try space("ABC")
    _ = await first.openSpaceChat(shared)
    say("Open in the first window", in: first)
    let id = try XCTUnwrap(first.saved.activeSessionID)

    let refused = await second.openChat(id)
    XCTAssertFalse(refused)
    XCTAssertEqual(second.error, "This chat is open in another window.")

    first.shutdown()
    second.error = nil
    let reopened = await second.openChat(id)
    XCTAssertTrue(reopened)
    XCTAssertEqual(second.saved.conversation.messages.map(\.text), ["Open in the first window"])
  }

  func testSettingsChangedInOneModelReachTheOthersAndStay() async throws {
    let archive = ChatArchive(directory: root)
    let hub = HarnessModel(directory: root, store: archive.handle(claimsChats: false))
    let open = window(archive)
    _ = await open.openSpaceChat(try space("ABC"))

    var profile = hub.saved.profile
    profile.name = "Settings from the hub"
    hub.updateProfile(profile)
    XCTAssertEqual(open.saved.profile.name, "Settings from the hub")

    // A later save from the window keeps the hub's change.
    say("Still talking", in: open)
    XCTAssertEqual(try StateStore(directory: root).load().profile.name, "Settings from the hub")
  }

  func testHubNeverClaimsOrOverwritesAWindowsChat() async throws {
    let archive = ChatArchive(directory: root)
    let hub = HarnessModel(directory: root, store: archive.handle(claimsChats: false))
    let open = window(archive)
    let shared = try space("ABC")
    _ = await open.openSpaceChat(shared)
    say("Window chat", in: open)
    hub.persist()
    say("Window chat continues", in: open)
    hub.persist()

    let disk = try StateStore(directory: root).load()
    let chat = try XCTUnwrap(disk.sessions?.first { $0.id == open.saved.activeSessionID })
    XCTAssertEqual(chat.conversation.messages.count, 2)
    XCTAssertEqual(disk.activeSessionID, open.saved.activeSessionID)
  }
}
