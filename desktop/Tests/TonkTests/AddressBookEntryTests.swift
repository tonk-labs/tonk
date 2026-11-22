import Foundation
import XCTest

@testable import HarnessCore
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
      ])

    XCTAssertEqual(entries.map(\.name), ["Board helper", "Claude Code", "Jonas's Cursor", "Mira"])
    let helper = entries[0]
    XCTAssertEqual(helper.kind, .agent)
    XCTAssertEqual(helper.grants.map(\.id), ["b", "a"])
    XCTAssertEqual(entries[1].id, "grant:d")
    XCTAssertNil(entries[1].did)
    XCTAssertEqual(entries[2].did, "did:key:zCursor")
    XCTAssertEqual(entries[3].kind, .person)
  }

  func testAnAgentLabelMarksAContactAsAnAgent() {
    let entries = AddressBookEntry.entries(
      contacts: [DesktopContact(id: "did:key:zBot", name: "Bot", labels: ["agent"])],
      grants: [])
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

  func testHostedAgentsJoinTheirKeyOrStandAlone() {
    let space = HostedAgentSpace(subject: "did:key:zSpace", key: "zSpace", access: "write")
    let entries = AddressBookEntry.entries(
      contacts: [DesktopContact(id: "did:key:zSaved", name: "My ChatGPT", labels: [])],
      grants: [grant("g", recipient: "did:key:zLinked", label: "Old label")],
      hosted: [
        HostedAgent(did: "did:key:zSaved", key: "zSaved", name: "ChatGPT", spaces: [space]),
        HostedAgent(did: "did:key:zLinked", key: "zLinked", name: "Claude", spaces: [space]),
        HostedAgent(did: "did:key:zNew", key: "zNew", name: nil, spaces: [space]),
      ])

    XCTAssertEqual(entries.map(\.name), ["Claude", "Connected agent", "My ChatGPT"])
    XCTAssertEqual(entries.map(\.kind), [.agent, .agent, .agent])
    XCTAssertEqual(entries.map(\.did), ["did:key:zLinked", "did:key:zNew", "did:key:zSaved"])
    // A saved contact keeps its own name and gains the agent's spaces.
    XCTAssertEqual(entries[2].hosted?.key, "zSaved")
    // An agent that also holds a grant is one row with both.
    XCTAssertEqual(entries[0].grants.map(\.id), ["g"])
    XCTAssertEqual(entries[0].hosted?.key, "zLinked")
    XCTAssertEqual(entries[1].hosted?.spaces, [space])
    XCTAssertTrue(entries[1].matches("zNew"))
  }

  func testSearchMatchesAKnownEmail() {
    let contact = DesktopContact(
      id: "did:key:zAsimov", name: "asimov", labels: [],
      emailIdentity: EmailContactIdentity(
        email: "isaac@example.com", rootDid: "did:key:zAsimov",
        directory: URL(string: "https://tonk.example")!, checkedAt: Date()))
    let entries = AddressBookEntry.entries(contacts: [contact], grants: [])
    XCTAssertTrue(entries[0].matches("isaac@"))
    XCTAssertTrue(entries[0].matches("example.com"))
  }
}
