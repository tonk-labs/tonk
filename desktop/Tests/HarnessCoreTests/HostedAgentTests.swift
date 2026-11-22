import XCTest

@testable import HarnessCore

final class HostedAgentTests: XCTestCase {
  /// Shaped like the worker's `AgentSummary`: optional fields serialize as null.
  func testDecodesTheAgentsListing() throws {
    let agents = try JSONDecoder().decode(
      [HostedAgent].self,
      from: Data(
        #"[{"did":"did:key:zAgent","key":"zAgent","name":"ChatGPT","spaces":[{"subject":"did:key:zTeam","key":"zTeam","name":"Team","access":"write","expiresAt":1800604800},{"subject":"did:key:zNotes","key":"zNotes","name":null,"access":"read","expiresAt":null}]},{"did":"did:key:zBare","key":"zBare","name":null,"spaces":[]}]"#
          .utf8))
    XCTAssertEqual(agents.map(\.did), ["did:key:zAgent", "did:key:zBare"])
    XCTAssertEqual(agents[0].displayName, "ChatGPT")
    XCTAssertEqual(agents[0].spaces.map(\.canWrite), [true, false])
    XCTAssertEqual(agents[0].spaces[0].expiresAt, 1_800_604_800)
    XCTAssertNil(agents[0].spaces[1].name)
    XCTAssertNil(agents[0].spaces[1].expiresAt)
    XCTAssertEqual(agents[1].displayName, "Connected agent")
    XCTAssertTrue(agents[1].spaces.isEmpty)
  }

  func testABlankNameFallsBack() {
    XCTAssertEqual(HostedAgent(did: "d", key: "k", name: "  ").displayName, "Connected agent")
  }
}
