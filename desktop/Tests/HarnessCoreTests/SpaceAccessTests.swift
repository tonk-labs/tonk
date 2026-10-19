import XCTest

@testable import HarnessCore

final class SpaceAccessTests: XCTestCase {
  let now = Date(timeIntervalSince1970: 1_800_000_000)
  let subject = "did:key:zSpace"

  func testDecodesWorkerListings() throws {
    let invites = try JSONDecoder().decode(
      [InviteSummary].self,
      from: Data(
        #"[{"targetCid":"bafy1","kind":"open","status":"active"},{"targetCid":"bafy2","kind":"scoped","recipientRoot":"did:key:zMira","status":"active","invitation":"tonk:inv"}]"#
          .utf8))
    XCTAssertEqual(invites[0].invitation, nil)
    XCTAssertEqual(invites[1].recipientRoot, "did:key:zMira")
    let agents = try JSONDecoder().decode(
      [AgentConnection].self,
      from: Data(
        #"[{"id":"g1","repo":"r","subject":"did:key:zSpace","recipient":"did:key:zA","label":"x","scope":"s","expiresAt":1800604800,"status":"active","confirmed":true,"installations":[{"id":"i","name":"Claude Code"}],"targets":[]}]"#
          .utf8))
    XCTAssertEqual(agents[0].installations.map(\.name), ["Claude Code"])
    XCTAssertTrue(agents[0].isLive)
  }

  func testLinksDescribeWhatEachGrantAllows() {
    let members = [
      SpaceMember(did: "did:key:zMira", name: "Mira", role: "tonk:member", invitation: "tonk:inv"),
      SpaceMember(did: "did:key:zLu", name: "Lu", role: "tonk:member", invitation: "tonk:inv"),
    ]
    let links = SpaceAccess.links(
      subject: subject,
      invites: [
        InviteSummary(targetCid: "bafy1", kind: "open", invitation: "tonk:inv"),
        InviteSummary(targetCid: "bafy2", kind: "open"),
        InviteSummary(targetCid: "bafy3", kind: "open", status: "unconfigured"),
      ],
      agents: [
        AgentConnection(
          id: "g1", subject: subject, expiresAt: now.timeIntervalSince1970 + 7 * 86_400),
        AgentConnection(id: "g2", subject: subject, expiresAt: 0, status: "revoked"),
        AgentConnection(id: "g3", subject: "did:key:zOther", expiresAt: 0),
      ],
      members: members, isPublic: true)
    XCTAssertEqual(
      links.map(\.id), ["publicRead:" + subject, "person:bafy1", "person:bafy2", "agent:g1"])
    XCTAssertEqual(links[0].detail(now: now), "Anyone · Read · Whole space · Never expires")
    XCTAssertEqual(links[1].detail(now: now), "Anyone · Write · Whole space · Never expires")
    XCTAssertEqual(links[1].usage(madeAt: nil, now: now), "Used by Mira and Lu")
    // An older worker cannot say who claimed it, so nothing is claimed either way.
    XCTAssertNil(links[2].usage(madeAt: nil, now: now))
    XCTAssertEqual(links[3].detail(now: now), "Anyone · Write · Whole space · Expires in 7 days")
    XCTAssertEqual(links[3].usage(madeAt: now, now: now), "Made just now · No registered clients")
    XCTAssertTrue(links[3].forAgents)
    XCTAssertTrue(links[1].canReset)
    XCTAssertFalse(links[3].canReset)
  }

  func testScopedInviteNamesItsRecipient() {
    let links = SpaceAccess.links(
      subject: subject,
      invites: [InviteSummary(targetCid: "c", kind: "scoped", recipientRoot: "did:key:zJonas")],
      agents: [],
      members: [SpaceMember(did: "did:key:zJonas", name: "Jonas", role: nil, invitation: nil)],
      isPublic: false)
    XCTAssertEqual(links[0].title, "Link for Jonas")
    XCTAssertEqual(links[0].detail(now: now), "Jonas · Write · Whole space · Never expires")
  }

  func testPeopleSummaryNamesTheFounder() {
    var access = SpaceAccess(
      members: [
        SpaceMember(did: "did:key:zKai", name: "Kai", role: "tonk:founder", invitation: nil),
        SpaceMember(did: "did:key:zMe", name: "Eileen", role: "tonk:member", invitation: nil),
      ],
      agents: [
        AgentConnection(id: "a", subject: subject, expiresAt: 0),
        AgentConnection(id: "b", subject: subject, expiresAt: 0, status: "revoked"),
      ],
      selfDid: "did:key:zMe")
    XCTAssertEqual(access.peopleSummary, "Kai’s space · 2 people, 1 agent")
    access.selfDid = "did:key:zKai"
    access.agents = []
    XCTAssertEqual(access.peopleSummary, "Your space · 2 people")
  }

  func testRelativeTimes() {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    let noon = Date(timeIntervalSince1970: 1_800_014_400)  // 12:00 UTC
    func ago(_ seconds: Double) -> String {
      AccessText.relative(noon.addingTimeInterval(-seconds), now: noon, calendar: calendar)
    }
    XCTAssertEqual(ago(10), "Just now")
    XCTAssertEqual(ago(4 * 60), "4 min ago")
    XCTAssertEqual(ago(3600), "1 hour ago")
    XCTAssertEqual(ago(2 * 3600), "2 hours ago")
    XCTAssertEqual(ago(20 * 3600), "Yesterday")
    XCTAssertEqual(ago(2 * 86_400), "2 days ago")
  }

  func testNameListsAndInitials() {
    XCTAssertEqual(AccessText.list(["Mira", "Lu"]), "Mira and Lu")
    XCTAssertEqual(AccessText.list(["A", "B", "C", "D", "E"]), "A, B and 3 others")
    XCTAssertEqual(AccessText.initials("Eileen Wagner"), "EW")
    XCTAssertEqual(AccessText.initials(""), "?")
  }

  func testExpiryWording() {
    XCTAssertEqual(
      AccessLink.expiry(now.addingTimeInterval(3 * 3600), status: "active", now: now),
      "Expires in 3 hours")
    XCTAssertEqual(
      AccessLink.expiry(now.addingTimeInterval(-1), status: "active", now: now), "Expired")
    XCTAssertEqual(AccessLink.expiry(nil, status: "revoked", now: now), "Revoked")
  }

  func testViewEntriesOpenTheirRoute() throws {
    let space = try XCTUnwrap(
      TonkSpace.decodeCatalog(Data(#"[{"subject":"did:key:zAbc","name":"x"}]"#.utf8)).first)
    let home = SpaceViewEntry(model: "tonk:board", title: "Board", isHome: true)
    let directory = SpaceViewEntry(model: "tonk:board", title: "Board", isHome: false)
    XCTAssertEqual(home.url(in: space), space.url)
    XCTAssertEqual(directory.url(in: space).lastPathComponent, "tonk:board")
    XCTAssertEqual(SpaceViewEntry.title(forModel: "id:cycle-14"), "Cycle 14")
    XCTAssertEqual(SpaceViewEntry.title(forModel: "tonk/person"), "Person")
  }

  func testJournalKeepsRecentEventsPerSpace() throws {
    var journal = AccessJournal()
    journal.record(
      AccessEvent(date: now, subject: subject, action: .made, kind: .agent, target: "g1"))
    journal.record(
      AccessEvent(
        date: now.addingTimeInterval(60), subject: "other", action: .made, kind: .person,
        target: "c"))
    journal.record(
      AccessEvent(
        date: now.addingTimeInterval(120), subject: subject, action: .revoked, kind: .agent,
        target: "g1"))
    XCTAssertEqual(
      journal.recent(subject: subject).map(\.sentence),
      ["You revoked a link for agents", "You made a link for agents"])
    XCTAssertEqual(journal.madeAt(target: "g1"), now)
    journal.record(
      AccessEvent(
        date: now.addingTimeInterval(180), subject: subject, action: .reset, kind: .person,
        target: "fresh"))
    XCTAssertEqual(journal.madeAt(target: "fresh"), now.addingTimeInterval(180))
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: url) }
    try journal.save(to: url)
    XCTAssertEqual(AccessJournal.load(from: url), journal)
  }

  func testSpaceContainsItsViews() throws {
    let space = try XCTUnwrap(
      TonkSpace.decodeCatalog(Data(#"[{"subject":"did:key:zAbc","name":"x"}]"#.utf8)).first)
    XCTAssertTrue(space.contains(space.url))
    XCTAssertTrue(space.contains(space.url.appendingPathComponent("id:board")))
    XCTAssertFalse(space.contains(URL(string: space.url.absoluteString + "Other")!))
    XCTAssertFalse(space.contains(URL(string: "https://elsewhere.example" + space.url.path)!))
  }

  func testRevocationSurvivesJournalTrimmingAndLegacyLoading() throws {
    var journal = AccessJournal(events: [
      AccessEvent(date: now, subject: subject, action: .revoked, kind: .person, target: "old")
    ])
    for _ in 0..<501 {
      journal.record(
        AccessEvent(date: now, subject: subject, action: .made, kind: .person, target: "new"))
    }
    let restored = try JSONDecoder().decode(AccessJournal.self, from: JSONEncoder().encode(journal))
    XCTAssertEqual(restored.revokedTargets[subject], ["old"])
    XCTAssertFalse(restored.events.contains { $0.target == "old" })
    let legacy = try JSONSerialization.data(withJSONObject: ["events": []])
    XCTAssertTrue(try JSONDecoder().decode(AccessJournal.self, from: legacy).revokedTargets.isEmpty)
  }
}
