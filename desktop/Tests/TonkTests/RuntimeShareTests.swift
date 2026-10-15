import XCTest

@testable import Tonk

@MainActor
final class RuntimeShareTests: XCTestCase {
  func testInvitationMustBeAnOpenJoinLinkOnThisDeployment() throws {
    let origin = URL(string: "https://staging.tonk.xyz")!
    let link = "https://staging.tonk.xyz/join#fixture"
    XCTAssertEqual(
      try RuntimeModel.validateShareLink(["kind": "open", "url": link], origin: origin)
        .absoluteString,
      link)
    for invalid in [
      "https://tonk.network/join#fixture", "https://staging.tonk.xyz/space/example",
      "https://staging.tonk.xyz/join", "http://staging.tonk.xyz/join#fixture",
      "https://user@staging.tonk.xyz/join#fixture",
    ] {
      XCTAssertThrowsError(
        try RuntimeModel.validateShareLink(["kind": "open", "url": invalid], origin: origin))
    }
    XCTAssertThrowsError(
      try RuntimeModel.validateShareLink(["kind": "scoped", "url": link], origin: origin))
  }
}
