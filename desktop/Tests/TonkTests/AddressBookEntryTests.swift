import HarnessCore
import XCTest

@testable import Tonk

final class AddressBookEntryTests: XCTestCase {
  private func grant(_ id: String, recipient: String?, label: String = "", expires: Double = 1)
    -> AgentConnection
  {
    AgentConnection(
      id: id, subject: "did:key:zSpace", label: label, expiresAt: expires, confirmed: true,
      recipient: recipient)
  }

  func testItListsPeopleAndAgentsTogether() {
    let entries = AddressBookEntry.entries(
      contacts: [
        DesktopContact(id: "did:key:zMira", name: "Mira", labels: []),
        DesktopContact(id: "did:key:zHelper", name: "Board helper", labels: []),
      ],
      grants: [
        grant("a", recipient: "did:key:zHelper", expires: 1),
        grant("b", recipient: "did:key:zHelper", expires: 2),
        grant("c", recipient: "did:key:zCursor", label: "Jonas's Cursor"),
        grant("d", recipient: nil, label: "Claude Code"),
      ],
      chatAgent: "Codex")

    XCTAssertEqual(
      entries.map(\.name), ["Codex", "Board helper", "Claude Code", "Jonas's Cursor", "Mira"])
    XCTAssertEqual(entries[0].kind, .chatAgent)
    let helper = entries[1]
    XCTAssertEqual(helper.kind, .agent)
    XCTAssertEqual(helper.grants.map(\.id), ["b", "a"])
    XCTAssertEqual(entries[2].id, "grant:d")
    XCTAssertNil(entries[2].did)
    XCTAssertEqual(entries[3].did, "did:key:zCursor")
    XCTAssertEqual(entries[4].kind, .person)
  }

  func testAnAgentLabelMarksAContactAsAnAgent() {
    let entries = AddressBookEntry.entries(
      contacts: [DesktopContact(id: "did:key:zBot", name: "Bot", labels: ["agent"])],
      grants: [], chatAgent: nil)
    XCTAssertEqual(entries.map(\.kind), [.agent])
  }

  func testSearchMatchesNameKeyAndClient() {
    let entry = AddressBookEntry(
      id: "did:key:zA", kind: .agent, name: "Helper",
      contact: DesktopContact(id: "did:key:zA", name: "Helper", labels: []),
      grants: [
        AgentConnection(
          id: "g", subject: "s", expiresAt: 0,
          installations: [.init(id: "i", name: "Claude Code")])
      ])
    XCTAssertTrue(entry.matches("help"))
    XCTAssertTrue(entry.matches("zA"))
    XCTAssertTrue(entry.matches("claude"))
    XCTAssertFalse(entry.matches("mira"))
  }
}
