import HarnessCore
import XCTest

@testable import Tonk

@MainActor
final class AccessModelTests: XCTestCase {
  func testRosterRowsBecomeMembersFounderFirst() {
    let members = AccessModel.members([
      ["this": "m1", "fields": ["member": "did:key:zLu", "name": "Lu", "invitation": "tonk:i"]],
      ["this": "m2", "fields": ["member": "did:key:zKai", "name": "Kai", "role": "tonk:founder"]],
      ["this": "m3", "fields": ["member": "did:key:zLu", "name": "Lu again"]],
      ["this": "m4", "fields": ["name": "no member"]],
    ])
    XCTAssertEqual(members.map(\.did), ["did:key:zKai", "did:key:zLu"])
    XCTAssertEqual(members[1].invitation, "tonk:i")
  }

  func testViewsAreTheHomeThenTheSpacesOwnDirectories() {
    let views = AccessModel.views(
      home: "id:board", directories: ["id:cycle-14", "tonk:member", "id:board", "id:cycle-14"])
    XCTAssertEqual(views.map(\.title), ["Board", "Cycle 14"])
    XCTAssertEqual(views.map(\.isHome), [true, false])
    // A space still on the blank canvas opens its home all the same.
    XCTAssertEqual(AccessModel.views(home: "tonk:blank", directories: []).map(\.title), ["Home"])
  }

  func testPublicAddressUsesTheSpacesHost() {
    XCTAssertEqual(
      AccessModel.publicAddress("https://sync.example/access/v1", subject: "did:key:zS"),
      "https://sync.example/space/did:key:zS")
    XCTAssertNil(AccessModel.publicAddress("ftp://sync.example", subject: "did:key:zS"))
    XCTAssertNil(AccessModel.publicAddress(nil, subject: "did:key:zS"))
  }
}
