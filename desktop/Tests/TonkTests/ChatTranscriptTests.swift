import HarnessCore
import XCTest

@testable import Tonk

final class ChatTranscriptTests: XCTestCase {
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
