import HarnessCore
import XCTest

@testable import Tonk

@MainActor
final class AccessActionsTests: XCTestCase {
  private func space() throws -> TonkSpace {
    try TonkSpace.decodeCatalog(Data(#"[{"subject":"did:key:zTest","name":"Verification"}]"#.utf8))[
      0]
  }

  func testResetDoesNotRevokeOldLinkWhenReplacementCannotBeCopied() async throws {
    let space = try space()
    let runtime = AccessRuntimeStub()
    let store = LinkStoreStub()
    let access = AccessModel(vault: store, copyToPasteboard: { _ in false })
    access.runtime = runtime
    await access.resetLink(
      AccessLink(kind: .person, subject: space.subject, target: "old", usedBy: []), space: space)
    XCTAssertEqual(runtime.revoked, [])
    XCTAssertTrue(access.notice?.contains("old link is still active") == true)
    XCTAssertFalse(access.notice?.contains("New link copied") == true)
    XCTAssertNotNil(store.values["replacement"])
  }

  func testResetReportsPartialFailureAndKeepsBothLinksWhenRevocationFails() async throws {
    let space = try space()
    let runtime = AccessRuntimeStub()
    runtime.revokeFails = true
    let store = LinkStoreStub()
    store.values["old"] = "old test link"
    var copied = false
    let access = AccessModel(
      vault: store,
      copyToPasteboard: { _ in
        copied = true
        return true
      })
    access.runtime = runtime
    await access.resetLink(
      AccessLink(kind: .person, subject: space.subject, target: "old", usedBy: []), space: space)
    XCTAssertTrue(copied)
    XCTAssertEqual(runtime.revoked, ["old"])
    XCTAssertNotNil(store.values["old"])
    XCTAssertNotNil(store.values["replacement"])
    XCTAssertTrue(access.notice?.contains("old link is still active") == true)
    XCTAssertEqual(access.journal.events.last?.action, .made)
  }

  func testSuccessfulResetCopiesBeforeRevokingAndRecordsReplacement() async throws {
    let space = try space()
    let runtime = AccessRuntimeStub()
    let store = LinkStoreStub()
    store.values["old"] = "old test link"
    let access = AccessModel(
      vault: store,
      copyToPasteboard: { _ in
        XCTAssertTrue(runtime.revoked.isEmpty)
        return true
      })
    access.runtime = runtime
    await access.resetLink(
      AccessLink(kind: .person, subject: space.subject, target: "old", usedBy: []), space: space)
    XCTAssertEqual(runtime.revoked, ["old"])
    XCTAssertNil(store.values["old"])
    XCTAssertNotNil(store.values["replacement"])
    XCTAssertEqual(access.journal.events.last?.action, .reset)
    XCTAssertTrue(access.journal.revokedTargets[space.subject]?.contains("old") == true)
    runtime.invites = [["targetCid": "old", "kind": "open", "status": "active"]]
    await access.refresh(space)
    XCTAssertTrue(access.links(for: space.subject).isEmpty)
  }

  func testConfirmedRevokeDoesNotReappearFromStaleReplica() async throws {
    let space = try space()
    let runtime = AccessRuntimeStub()
    runtime.invites = [["targetCid": "old", "kind": "open", "status": "active"]]
    let access = AccessModel(vault: LinkStoreStub())
    access.runtime = runtime
    await access.refresh(space)
    XCTAssertEqual(access.links(for: space.subject).count, 1)
    await access.revoke(
      AccessLink(kind: .person, subject: space.subject, target: "old", usedBy: []), space: space)
    XCTAssertTrue(access.links(for: space.subject).isEmpty)
  }

  func testKeychainFailureIsReportedAfterMinting() async throws {
    let store = LinkStoreStub()
    store.failWrites = true
    let access = AccessModel(vault: store, copyToPasteboard: { _ in true })
    let runtime = AccessRuntimeStub()
    access.runtime = runtime
    let made = await access.make(.person, in: try space())
    XCTAssertTrue(made)
    XCTAssertTrue(access.notice?.contains("Keychain") == true)
  }

  func testDeniedAccessActionsNeverReportSuccessOrRecordActivity() async throws {
    let space = try space()
    let runtime = AccessRuntimeStub()
    runtime.permissionDenied = true
    let store = LinkStoreStub()
    let access = AccessModel(
      vault: store,
      copyToPasteboard: { _ in
        XCTFail("Denied actions must not copy a link")
        return true
      })
    access.runtime = runtime
    for kind in AccessModel.NewLink.allCases {
      let made = await access.make(kind, in: space)
      XCTAssertFalse(made)
      XCTAssertTrue(access.notice?.contains("Permission denied") == true)
      XCTAssertTrue(access.working.isEmpty)
      XCTAssertTrue(access.journal.events.isEmpty)
      XCTAssertTrue(store.values.isEmpty)
    }
    await access.revoke(
      AccessLink(kind: .person, subject: space.subject, target: "denied", usedBy: []), space: space)
    XCTAssertTrue(access.notice?.contains("Permission denied") == true)
    XCTAssertTrue(access.journal.events.isEmpty)
    XCTAssertTrue(access.journal.revokedTargets.isEmpty)
  }

  func testActivitySyncMergesTheAccountsEventsAndUploadsThisMacsLegacyJournal() async throws {
    let space = try space()
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let date = Date(timeIntervalSince1970: 1_800_000_000)
    let mine = AccessEvent(
      date: date, subject: space.subject, action: .made, kind: .person, target: "mine")
    let elsewhere = AccessEvent(
      date: date, subject: "did:key:zOtherAccount", action: .made, kind: .person, target: "x")
    // A journal from before activity synced: no `unsynced` key.
    let legacy = #"{"events":["# + [mine, elsewhere].map { event in
      let encoder = JSONEncoder()
      encoder.dateEncodingStrategy = .iso8601
      return String(decoding: try! encoder.encode(event), as: UTF8.self)
    }.joined(separator: ",") + "]}"
    try Data(legacy.utf8).write(to: directory.appendingPathComponent("access-journal.json"))

    let runtime = AccessRuntimeStub()
    let theirs = UUID()
    runtime.remoteEvents = [
      [
        "this": "urn:uuid:" + theirs.uuidString.lowercased(), "space": space.subject,
        "action": "reset", "kind": "person", "at": 1_800_000_060_000.0, "target": "new",
        "replaced": "old",
      ],
      ["this": "tonk:not-an-event", "space": space.subject, "action": "made", "kind": "person"],
    ]
    let access = AccessModel(vault: LinkStoreStub())
    access.prepare(directory: directory, runtime: runtime)
    access.setCatalog([space])
    await access.syncActivity()

    XCTAssertEqual(
      access.journal.recent(subject: space.subject).map(\.id), [theirs, mine.id])
    XCTAssertEqual(access.journal.revokedTargets[space.subject], ["old"])
    // Only this account's space is uploaded; the other account's event waits.
    XCTAssertEqual(runtime.recorded.map { $0["this"] as? String }, [mine.entity])
    XCTAssertEqual(runtime.recorded.first?["at"] as? Double, 1_800_000_000_000)
    XCTAssertEqual(access.journal.pending.map(\.id), [elsewhere.id])
    let saved = AccessJournal.load(from: directory.appendingPathComponent("access-journal.json"))
    XCTAssertEqual(saved.pending.map(\.id), [elsewhere.id])
    XCTAssertTrue(saved.events.contains { $0.id == theirs })
  }

  func testResetRecordsTheReplacedLinkForOtherDesktops() async throws {
    let space = try space()
    let runtime = AccessRuntimeStub()
    let access = AccessModel(vault: LinkStoreStub(), copyToPasteboard: { _ in true })
    access.runtime = runtime
    await access.resetLink(
      AccessLink(kind: .person, subject: space.subject, target: "old", usedBy: []), space: space)
    await access.flushActivity()
    let row = try XCTUnwrap(runtime.recorded.last)
    XCTAssertEqual(row["action"] as? String, "reset")
    XCTAssertEqual(row["target"] as? String, "replacement")
    XCTAssertEqual(row["replaced"] as? String, "old")
    XCTAssertTrue(access.journal.pending.isEmpty)
  }

  func testAccountRefreshClearsRecoveredError() async {
    let access = AccessModel(vault: LinkStoreStub())
    let runtime = AccessRuntimeStub()
    access.runtime = runtime
    runtime.accountFails = true
    await access.refreshAccount()
    XCTAssertNotNil(access.errors["account"])
    runtime.accountFails = false
    await access.refreshAccount()
    XCTAssertNil(access.errors["account"])
  }

  func testPartialSpaceReadShowsWarningAndRefreshClearsIt() async throws {
    let access = AccessModel(vault: LinkStoreStub())
    let runtime = AccessRuntimeStub()
    access.runtime = runtime
    let space = try space()
    runtime.spaceWarnings = ["Links could not be loaded. Try refreshing."]
    await access.refresh(space)
    XCTAssertNotNil(access.spaces[space.id])
    XCTAssertEqual(access.errors[space.id], runtime.spaceWarnings[0])
    runtime.spaceWarnings = []
    await access.refresh(space)
    XCTAssertNil(access.errors[space.id])
  }

  func testResetDuringAccountReadDoesNotContinueLoadingPreviousCatalog() async throws {
    let access = AccessModel(vault: LinkStoreStub())
    let runtime = AccessRuntimeStub()
    access.runtime = runtime
    let space = try space()
    let entered = expectation(description: "Account read suspended")
    var resume: CheckedContinuation<Void, Never>?
    runtime.accountRead = {
      await withCheckedContinuation { continuation in
        resume = continuation
        entered.fulfill()
      }
    }
    let refresh = Task { await access.refreshAll([space]) }
    await fulfillment(of: [entered], timeout: 1)
    access.notice = "Previous account notice"
    access.reset()
    resume?.resume()
    await refresh.value
    XCTAssertEqual(runtime.spaceReads, 0)
    XCTAssertTrue(access.spaces.isEmpty)
    XCTAssertTrue(access.loading.isEmpty)
    XCTAssertNil(access.accountName)
    XCTAssertNil(access.notice)
  }

  func testAccountChangeDuringResetDoesNotCopyOrRevokeForNextAccount() async throws {
    let runtime = AccessRuntimeStub()
    let store = LinkStoreStub()
    let access = AccessModel(
      vault: store,
      copyToPasteboard: { _ in
        XCTFail("A previous account's link must not be copied")
        return true
      })
    access.runtime = runtime
    let space = try space()
    let entered = expectation(description: "Mint suspended")
    var resume: CheckedContinuation<Void, Never>?
    runtime.mintRead = {
      await withCheckedContinuation { continuation in
        resume = continuation
        entered.fulfill()
      }
    }
    let operation = Task {
      await access.resetLink(
        AccessLink(kind: .person, subject: space.subject, target: "old", usedBy: []), space: space)
    }
    await fulfillment(of: [entered], timeout: 1)
    access.reset()
    resume?.resume()
    await operation.value
    XCTAssertTrue(runtime.revoked.isEmpty)
    XCTAssertTrue(store.values.isEmpty)
    XCTAssertTrue(access.working.isEmpty)
    XCTAssertTrue(access.journal.events.isEmpty)
    XCTAssertNil(access.notice)
  }
}

private final class LinkStoreStub: AccessLinkStore {
  var values: [String: String] = [:]
  var failWrites = false
  func read(_ target: String) -> String? { values[target] }
  func write(_ link: String, for target: String) throws {
    if failWrites { throw HarnessError.message("Keychain unavailable") }
    values[target] = link
  }
  func remove(_ target: String) { values[target] = nil }
}

@MainActor
private final class AccessRuntimeStub: SpaceAccessRuntime {
  var catalogBranch: String? = "main"
  var revoked: [String] = []
  var revokeFails = false
  var accountFails = false
  var permissionDenied = false
  var accountRead: (() async -> Void)?
  var mintRead: (() async -> Void)?
  var spaceWarnings: [String] = []
  var invites: [[String: Any]] = []
  var spaceReads = 0
  var remoteEvents: [[String: Any]] = []
  var recorded: [[String: Any]] = []
  func createCLILink(_ space: TonkSpace) async throws -> String {
    if permissionDenied { throw HarnessError.message("Permission denied") }
    return "unused"
  }
  func accountScript(_ script: String, arguments: [String: Any]) async throws -> [String: Any] {
    if permissionDenied { throw HarnessError.message("Permission denied") }
    if script == AccessModel.accountScript {
      await accountRead?()
      if accountFails { throw HarnessError.message("Offline") }
      return ["connections": [], "name": "Verifier", "root": "did:key:zVerifier"]
    }
    if script == AccessModel.activityScript {
      XCTAssertEqual(arguments["branch"] as? String, catalogBranch)
      return ["events": remoteEvents]
    }
    if script == AccessModel.recordScript {
      let events = arguments["events"] as? [[String: Any]] ?? []
      recorded += events
      return ["saved": events.count]
    }
    if script == AccessModel.spaceScript {
      spaceReads += 1
      return ["invites": invites, "members": [], "warnings": spaceWarnings]
    }
    if script.contains("const minted") {
      await mintRead?()
      return [
        "kind": "open", "url": "https://tonk.network/space/did:key:zTest#synthetic-test",
        "targets": ["replacement"],
      ]
    }
    if script.contains("/revoke") {
      revoked.append(arguments["target"] as! String)
      if revokeFails { throw HarnessError.message("Offline") }
      return ["status": "revoked"]
    }
    throw HarnessError.message("Unexpected runtime operation")
  }
}
