import HarnessCore
import XCTest

@testable import Tonk

final class ChatTranscriptTests: XCTestCase {
  func testActivityStaysAtTheStartOfEachTurn() throws {
    var conversation = Conversation()
    conversation.messages.append(ChatMessage(id: "u1", role: "user", text: "Build"))
    conversation.appendToolActivity("Reading space", messageID: "u1")
    conversation.appendDelta(itemID: "a1", text: "Working")
    conversation.appendToolActivity("Updating space", messageID: "u1")
    conversation.messages.append(ChatMessage(id: "u2", role: "user", text: "Fix filters"))
    XCTAssertNil(conversation.messages.last?.toolActivity)
    conversation.appendToolActivity("Inspecting preview", messageID: "u2")
    conversation.appendDelta(itemID: "a2", text: "Fixed")
    let restored = try JSONDecoder().decode(
      Conversation.self, from: JSONEncoder().encode(conversation))
    let groups = ChatTranscript.grouped(restored.messages)
    XCTAssertEqual(groups.map(\.id), ["u1", "a1", "u2", "a2"])
    XCTAssertEqual(groups[0].toolActivity, ["Reading space", "Updating space"])
    XCTAssertEqual(groups[2].toolActivity, ["Inspecting preview"])
    XCTAssertNil(groups[1].toolActivity)
    XCTAssertNil(groups[3].toolActivity)
  }

  func testLateErrorsAndCancellationRemainOnOriginatingTurn() throws {
    var conversation = Conversation()
    conversation.messages = [
      ChatMessage(id: "a", role: "user", text: "First"),
      ChatMessage(id: "b", role: "user", text: "Second"),
    ]
    conversation.appendToolActivity(
      "Cancelled; write outcome unknown. Query first.", messageID: "a")
    conversation.appendToolActivity("Late tool error", messageID: "a")
    conversation.appendToolActivity("Wrong conversation", messageID: "missing")
    XCTAssertEqual(conversation.messages[0].toolActivity?.count, 2)
    XCTAssertNil(conversation.messages[1].toolActivity)
    XCTAssertEqual(
      try JSONDecoder().decode(
        Conversation.self,
        from: JSONEncoder().encode(conversation)), conversation)
  }

  func testHistoryWithoutActivityStillDecodes() throws {
    let message = try JSONDecoder().decode(
      ChatMessage.self, from: Data(#"{"id":"old","role":"user","text":"Hello"}"#.utf8))
    XCTAssertNil(message.toolActivity)
  }

  func testConsecutiveRepliesShareSelectionWithoutCrossingUserMessages() {
    let messages = [
      ChatMessage(id: "u1", role: "user", text: "Question"),
      ChatMessage(id: "a1", role: "assistant", text: "First paragraph."),
      ChatMessage(id: "a2", role: "assistant", text: "Second paragraph."),
      ChatMessage(id: "u2", role: "user", text: "Follow-up"),
      ChatMessage(id: "a3", role: "assistant", text: "New reply."),
    ]
    let groups = ChatTranscript.grouped(messages)
    XCTAssertEqual(groups.map(\.id), ["u1", "a1", "u2", "a3"])
    XCTAssertEqual(groups[1].text, "First paragraph.\n\nSecond paragraph.")
    XCTAssertEqual(messages[1].text, "First paragraph.")
    var streaming = messages
    streaming[2].text += " More."
    XCTAssertEqual(ChatTranscript.grouped(streaming)[1].id, "a1")
    XCTAssertTrue(ChatTranscript.grouped(streaming)[1].text.hasSuffix(" More."))
    XCTAssertTrue(ChatTranscript.grouped([]).isEmpty)
  }
}
