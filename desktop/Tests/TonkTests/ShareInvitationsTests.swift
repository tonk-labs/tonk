import XCTest

@testable import Tonk

@MainActor
final class ShareInvitationsTests: XCTestCase {
  func testFirstClickCreatesAndCopiesEachRecipientAndLaterClicksReuseLinks() async {
    let sharing = ShareInvitations()
    var personCalls = 0
    var agentCalls = 0
    var clipboard: [String] = []
    for recipient in [ShareRecipient.person, .agent, .person, .agent] {
      let copied = await sharing.copyLink(
        for: recipient,
        personLink: {
          personCalls += 1
          return "person#complete"
        },
        agentLink: {
          agentCalls += 1
          return "agent#complete"
        },
        copy: { link in
          XCTAssertFalse(sharing.showingProgress, "Fast copies must not flash a spinner")
          clipboard.append(link)
          return true
        })
      XCTAssertTrue(copied)
      XCTAssertTrue(sharing[recipient].copied)
      XCTAssertFalse(sharing.showingProgress)
    }
    XCTAssertEqual(personCalls, 1)
    XCTAssertEqual(agentCalls, 1)
    XCTAssertEqual(
      clipboard, ["person#complete", "agent#complete", "person#complete", "agent#complete"])
  }

  func testSlowCreationShowsProgressAndCopiesOnCompletion() async {
    let sharing = ShareInvitations()
    let copied = await sharing.copyLink(
      for: .person,
      personLink: {
        XCTAssertFalse(sharing.showingProgress)
        try await Task.sleep(for: .milliseconds(350))
        XCTAssertTrue(sharing.showingProgress)
        return "person"
      },
      agentLink: {
        XCTFail("Wrong recipient")
        return ""
      },
      copy: { $0 == "person" })
    XCTAssertTrue(copied)
    XCTAssertFalse(sharing.creating)
    XCTAssertFalse(sharing.showingProgress)
  }

  func testClipboardFailureCanRetryWithoutMintingAgain() async {
    let sharing = ShareInvitations()
    let first = await sharing.copyLink(
      for: .agent,
      personLink: {
        XCTFail("Wrong recipient")
        return ""
      },
      agentLink: { "agent" }, copy: { _ in false })
    XCTAssertFalse(first)
    XCTAssertFalse(sharing[.agent].copied)
    XCTAssertFalse(sharing[.agent].failed)
    let second = await sharing.copyLink(
      for: .agent,
      personLink: {
        XCTFail("Must reuse the link")
        return ""
      },
      agentLink: {
        XCTFail("Must reuse the link")
        return ""
      }, copy: { $0 == "agent" })
    XCTAssertTrue(second)
  }

  func testUncertainMintDoesNotCopyOrRetryOrBlockOtherRecipient() async {
    let sharing = ShareInvitations()
    var attempts = 0
    for _ in 0..<2 {
      let copied = await sharing.copyLink(
        for: .agent,
        personLink: {
          XCTFail("Wrong recipient")
          return ""
        },
        agentLink: {
          attempts += 1
          throw CocoaError(.fileReadUnknown)
        },
        copy: { _ in
          XCTFail("Must not copy after failed creation")
          return true
        })
      XCTAssertFalse(copied)
    }
    XCTAssertEqual(attempts, 1)
    XCTAssertTrue(sharing[.agent].failed)
    XCTAssertNil(sharing[.agent].link)
    XCTAssertFalse(sharing.creating)
    let copied = await sharing.copyLink(
      for: .person, personLink: { "person" },
      agentLink: {
        XCTFail("Wrong recipient")
        return ""
      }, copy: { $0 == "person" })
    XCTAssertTrue(copied)
  }

  func testInFlightCreationRejectsAnotherCopyRequest() async {
    let sharing = ShareInvitations()
    let copied = await sharing.copyLink(
      for: .person,
      personLink: {
        XCTAssertTrue(sharing.creating)
        let duplicate = await sharing.copyLink(
          for: .agent,
          personLink: {
            XCTFail("Duplicate request")
            return ""
          },
          agentLink: {
            XCTFail("Concurrent request")
            return ""
          },
          copy: { _ in
            XCTFail("Concurrent copy")
            return true
          })
        XCTAssertFalse(duplicate)
        return "person"
      },
      agentLink: {
        XCTFail("Wrong recipient")
        return ""
      }, copy: { $0 == "person" })
    XCTAssertTrue(copied)
    XCTAssertNil(sharing[.agent].link)
    XCTAssertFalse(sharing.creating)
  }
}
