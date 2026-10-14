import Foundation
import XCTest

@testable import HarnessCore

final class SpaceProposalTests: XCTestCase {
  func testProposalRejectsUnexpectedInputsAndInvalidNames() throws {
    for value: JSONValue in [
      .object(["name": .string(""), "reason": .string("Keep our plan")]),
      .object(["name": .string("Dinner\nInjected"), "reason": .string("Keep our plan")]),
      .object(["name": .string("Dinner"), "reason": .string("")]),
      .object(["name": .string("Dinner"), "reason": .string("Plan"), "subject": .string("other")]),
    ] { XCTAssertThrowsError(try SpaceProposal(arguments: value)) }
    let proposal = try makeProposal()
    XCTAssertEqual(proposal.name, "Dinner")
    XCTAssertFalse(proposal.submitted)
    XCTAssertNotEqual(proposal.id, try makeProposal().id)
  }

  func testOnlyExactCreatedReceiptCanSelectSpace() throws {
    let proposal = try makeProposal()
    XCTAssertNil(try proposal.createdSpace(status: "pending", detail: ""))
    XCTAssertThrowsError(try proposal.createdSpace(status: "failed", detail: "Creation failed"))
    for path in [
      "https://other.test/space/did:key:zABC", "/space/did:key:zABC?target=other",
      "/space/not-a-did", "/space/did:key:zABC/other",
    ] {
      XCTAssertThrowsError(try proposal.createdSpace(status: "created", detail: path))
    }
    let space = try proposal.createdSpace(status: "created", detail: "/space/did:key:zABC")
    XCTAssertEqual(space?.subject, "did:key:zABC")
  }

  func testPersistedRequestSurvivesRestartAndAttachmentPreservesConversation() throws {
    var saved = SavedState()
    saved.conversation.threadID = "existing-thread"
    saved.conversation.messages = [ChatMessage(role: "user", text: "Plan dinner")]
    var proposal = try makeProposal()
    proposal.submitted = true
    proposal.branch = "main"
    proposal.account = "did:key:zAccount"
    saved.conversation.spaceProposal = proposal
    var restored = try JSONDecoder().decode(SavedState.self, from: JSONEncoder().encode(saved))
    XCTAssertEqual(restored, saved)
    let space = try XCTUnwrap(
      proposal.createdSpace(status: "created", detail: "/space/did:key:zABC"))
    XCTAssertThrowsError(try restored.conversation.attachCreatedSpace(space, proposalID: "other"))
    XCTAssertEqual(restored, saved)
    try restored.conversation.attachCreatedSpace(space, proposalID: proposal.id)
    XCTAssertNil(restored.conversation.spaceProposal)
    XCTAssertThrowsError(
      try restored.conversation.attachCreatedSpace(space, proposalID: proposal.id))
    XCTAssertEqual(restored.conversation.threadID, "existing-thread")
    XCTAssertEqual(restored.conversation.messages, saved.conversation.messages)
    let legacy = Data("{\"messages\":[],\"threadID\":\"legacy\"}".utf8)
    XCTAssertNil(try JSONDecoder().decode(Conversation.self, from: legacy).spaceProposal)
  }

  private func makeProposal() throws -> SpaceProposal {
    try SpaceProposal(
      arguments: .object(["name": .string(" Dinner "), "reason": .string("Keep our plan")]))
  }
}
