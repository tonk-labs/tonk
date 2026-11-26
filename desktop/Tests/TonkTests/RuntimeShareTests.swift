import XCTest

@testable import Tonk

@MainActor
final class RuntimeShareTests: XCTestCase {
  let origin = URL(string: "https://tonk.spot")!
  let subject = "did:key:zSpace"

  private func validate(_ url: String, kind: String = "open", origins: [URL]? = nil) throws
    -> String
  {
    try RuntimeModel.validateShareLink(
      ["kind": kind, "url": url], subject: subject, origins: origins ?? [origin]
    ).absoluteString
  }

  func testInvitationIsAnOpenTicketOrJoinLinkOnAKnownHost() throws {
    for link in [
      "https://tonk.spot/space/did:key:zSpace#seed",
      "https://tonk.spot/join#fixture",
      "https://tonk.spot/join?access=chain&ref=space",
    ] {
      XCTAssertEqual(try validate(link), link)
    }
    let sync = URL(string: "https://sync.tonk.example/access")!
    XCTAssertNoThrow(
      try validate("https://sync.tonk.example/space/did:key:zSpace#seed", origins: [origin, sync]))
    for invalid in [
      "https://tonk.network/join#fixture", "https://tonk.spot/space/did:key:zOther#seed",
      "https://tonk.spot/space/did:key:zSpace", "https://tonk.spot/join",
      "http://tonk.spot/join#fixture", "https://user@tonk.spot/join#fixture",
    ] {
      XCTAssertThrowsError(try validate(invalid), invalid)
    }
    XCTAssertThrowsError(try validate("https://tonk.spot/join#fixture", kind: "scoped"))
  }

  func testALinkForOneAccountIsAScopedJoinLinkNamingThatAccount() throws {
    func scoped(_ url: String, kind: String = "scoped", root: String = "did:key:zAda") throws
      -> URL
    {
      try RuntimeModel.validateShareLink(
        ["kind": kind, "url": url, "recipientRoot": root], subject: subject, origins: [origin],
        recipient: "did:key:zAda")
    }
    let link = "https://tonk.spot/join?access=chain&ref=space"
    XCTAssertEqual(try scoped(link).absoluteString, link)
    XCTAssertThrowsError(try scoped(link, kind: "open"), "an open link is not for one account")
    XCTAssertThrowsError(try scoped(link, root: "did:key:zOther"), "addressed to someone else")
    for invalid in [
      "https://tonk.spot/join#seed", "https://tonk.spot/space/did:key:zSpace#seed",
      "https://tonk.network/join?access=chain",
    ] {
      XCTAssertThrowsError(try scoped(invalid), invalid)
    }
  }
}
