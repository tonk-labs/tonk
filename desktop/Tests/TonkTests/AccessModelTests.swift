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
    XCTAssertFalse(members[1].isAgent)
  }

  func testRosterRowsCarryKindAndInviter() {
    let members = AccessModel.members([
      [
        "this": "m1",
        "fields": [
          "member": "did:key:zBot", "name": "Standup Bot", "role": "tonk:member",
          "inviter": "did:key:zKai", "kind": "tonk:agent",
        ],
      ]
    ])
    XCTAssertEqual(members[0].inviter, "did:key:zKai")
    XCTAssertEqual(members[0].kind, "tonk:agent")
    XCTAssertTrue(members[0].isAgent)
  }

  func testViewsAreTheHomeThenTheSpacesOwnDirectoriesByName() {
    let views = AccessModel.views(
      home: ["model": "id:card", "name": "card"],
      directories: [
        ["model": "concept:Bf4r", "name": "task-list"], ["model": "id:card", "name": "card"],
        ["model": "concept:Zz", "name": NSNull()],
      ])
    XCTAssertEqual(views.map(\.title), ["Card", "Task list", "Zz"])
    XCTAssertEqual(views.map(\.isHome), [true, false, false])
    // A space still on the blank canvas opens its home all the same.
    XCTAssertEqual(
      AccessModel.views(home: ["model": "tonk:blank"], directories: []).map(\.title), ["Home"])
    XCTAssertEqual(AccessModel.views(home: nil, directories: nil).map(\.title), ["Home"])
  }

  func testPublicAddressUsesTheSpacesHost() {
    XCTAssertEqual(
      AccessModel.publicAddress("https://sync.example/access/v1", subject: "did:key:zS"),
      "https://sync.example/space/did:key:zS")
    XCTAssertNil(AccessModel.publicAddress("ftp://sync.example", subject: "did:key:zS"))
    XCTAssertNil(AccessModel.publicAddress(nil, subject: "did:key:zS"))
  }

  func testLinksFollowTheCatalogAndResetForgetsTheAccount() throws {
    let kept = "did:key:zKept"
    let gone = "did:key:zGone"
    let link = { (subject: String) in
      SpaceAccess(links: [AccessLink(kind: .person, subject: subject, target: subject, usedBy: [])])
    }
    let access = AccessModel()
    access.showFixture(
      [kept: link(kept), gone: link(gone)], agents: [], accountName: "A", journal: AccessJournal())
    XCTAssertEqual(access.allLinks.count, 2)
    let catalog = try TonkSpace.decodeCatalog(
      Data(#"[{"subject":"did:key:zKept","name":"k"}]"#.utf8))
    access.setCatalog(catalog)
    XCTAssertEqual(access.allLinks.map(\.subject), [kept])
    XCTAssertNil(access.spaces[gone])
    XCTAssertTrue(access.loadedAll)
    access.reset()
    XCTAssertTrue(access.allLinks.isEmpty)
    XCTAssertNil(access.accountName)
  }
}
