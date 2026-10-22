import HarnessCore
import XCTest

@testable import Tonk

@MainActor
final class AccessActionsTests: XCTestCase {
  private func space() throws -> TonkSpace {
    try TonkSpace.decodeCatalog(Data(#"[{"subject":"did:key:zTest","name":"Verification"}]"#.utf8))[
      0]
  }

  func testCopyDefaultReusesRetainedGrantWithoutMinting() async throws {
    let space = try space()
    let store = LinkStoreStub()
    store.values["retained"] = "synthetic retained link"
    var copies: [String] = []
    let access = AccessModel(
      vault: store,
      copyToPasteboard: {
        copies.append($0)
        return true
      })
    access.showFixture(
      [
        space.id: SpaceAccess(links: [
          AccessLink(kind: .person, subject: space.id, target: "missing", usedBy: nil),
          AccessLink(kind: .person, subject: space.id, target: "retained", usedBy: nil),
        ])
      ], agents: [], accountName: "Tester", journal: AccessJournal())
    let copied = await access.copyDefaultLink(in: space)
    XCTAssertTrue(copied)
    XCTAssertEqual(copies, ["synthetic retained link"])
    XCTAssertTrue(access.journal.events.isEmpty)
  }

  func testMissingDefaultSecretRequiresExplicitReset() async throws {
    let space = try space()
    let access = AccessModel(
      vault: LinkStoreStub(),
      copyToPasteboard: { _ in
        XCTFail("No secret")
        return false
      })
    access.showFixture(
      [
        space.id: SpaceAccess(links: [
          AccessLink(kind: .person, subject: space.id, target: "missing", usedBy: nil)
        ])
      ], agents: [], accountName: "Tester", journal: AccessJournal())
    let copied = await access.copyDefaultLink(in: space)
    XCTAssertFalse(copied)
    XCTAssertTrue(access.notice?.contains("Reset") == true)
    XCTAssertTrue(access.journal.events.isEmpty)
  }

  func testNewDefaultDoesNotReportCopiedWhenPasteboardFails() async throws {
    let runtime = AccessRuntimeStub()
    let store = LinkStoreStub()
    let access = AccessModel(vault: store, copyToPasteboard: { _ in false })
    access.runtime = runtime
    let copied = await access.copyDefaultLink(in: try space())
    XCTAssertFalse(copied)
    XCTAssertNotNil(store.values["replacement"])
    XCTAssertTrue(access.notice?.contains("couldn’t be copied") == true)
  }

  /// Swaps in a telemetry instance that only queues, so a test can read what one click records.
  private func recording() throws -> Telemetry {
    let defaults = try XCTUnwrap(UserDefaults(suiteName: "AccessTelemetry-\(UUID().uuidString)"))
    let telemetry = Telemetry(
      environment: ["TONK_POSTHOG_KEY": "phc_test", "TONK_POSTHOG_ENDPOINT": "http://127.0.0.1:9"],
      defaults: defaults)
    try XCTSkipUnless(telemetry.isAvailable, "Telemetry is unavailable against a local runtime.")
    let previous = Telemetry.shared
    Telemetry.shared = telemetry
    addTeardownBlock { @MainActor in Telemetry.shared = previous }
    return telemetry
  }

  private func attempts(_ telemetry: Telemetry) -> [[String: JSONValue]] {
    telemetry.pending.filter { $0.name == "product_event" }.map(\.properties)
  }

  func testCopyingAnExistingDefaultLinkIsOneCopyAttempt() async throws {
    let telemetry = try recording()
    let space = try space()
    let store = LinkStoreStub()
    store.values["retained"] = "synthetic retained link"
    let access = AccessModel(vault: store, copyToPasteboard: { _ in true })
    access.showFixture(
      [
        space.id: SpaceAccess(links: [
          AccessLink(kind: .person, subject: space.id, target: "retained", usedBy: nil)
        ])
      ], agents: [], accountName: "Tester", journal: AccessJournal())
    _ = await access.copyDefaultLink(in: space, surface: .workspace)
    let events = attempts(telemetry)
    XCTAssertEqual(events.map { $0["phase"] }, [.string("started"), .string("finished")])
    XCTAssertEqual(Set(events.compactMap { $0["action"]?.string }), ["copy_share_link"])
    XCTAssertEqual(events.last?["result"], .string("success"))
    XCTAssertEqual(events.last?["stage"], .string("clipboard"))
    XCTAssertEqual(events.last?["surface"], .string("workspace"))
  }

  func testMakingTheDefaultLinkOnCopyIsStillOneAttempt() async throws {
    let telemetry = try recording()
    let runtime = AccessRuntimeStub()
    let access = AccessModel(vault: LinkStoreStub(), copyToPasteboard: { _ in true })
    access.runtime = runtime
    _ = await access.copyDefaultLink(in: try space())
    let events = attempts(telemetry)
    XCTAssertEqual(events.count, 2, "one started and one finished, not a nested mint attempt")
    XCTAssertEqual(Set(events.compactMap { $0["attempt_id"]?.string }).count, 1)
    XCTAssertEqual(events.last?["result"], .string("success"))
  }

  func testAMissingDefaultSecretIsABlockedCopy() async throws {
    let telemetry = try recording()
    let space = try space()
    let access = AccessModel(vault: LinkStoreStub(), copyToPasteboard: { _ in true })
    access.showFixture(
      [
        space.id: SpaceAccess(links: [
          AccessLink(kind: .person, subject: space.id, target: "missing", usedBy: nil)
        ])
      ], agents: [], accountName: "Tester", journal: AccessJournal())
    _ = await access.copyDefaultLink(in: space)
    XCTAssertEqual(attempts(telemetry).last?["result"], .string("blocked"))
    XCTAssertEqual(attempts(telemetry).last?["failure_kind"], .string("local_state"))
  }

  func testAddressBookPersistsByAccountAndResetHidesIt() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let runtime = AccessRuntimeStub()
    let access = AccessModel(vault: LinkStoreStub())
    access.prepare(directory: directory, runtime: runtime)
    await access.refreshAccount()
    let contact = DesktopContact(id: "did:key:zContact", name: "Contact", labels: ["Team"])
    try access.saveContact(contact)
    let restored = AccessModel(vault: LinkStoreStub())
    restored.prepare(directory: directory, runtime: runtime)
    XCTAssertTrue(restored.contacts.isEmpty)
    await restored.refreshAccount()
    XCTAssertEqual(restored.contacts, [contact])
    restored.reset()
    XCTAssertTrue(restored.contacts.isEmpty)
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

  func testLocalPasskeyMetadataIsOptionalAndClearedWithAccount() async throws {
    let access = AccessModel(vault: LinkStoreStub())
    let runtime = AccessRuntimeStub()
    access.runtime = runtime
    runtime.passkey = ["createdOn": "MacBook", "createdAt": 1_700_000_000.0]
    runtime.accountDetails = [
      "accountDetailsLoaded": true, "email": "test@example.test",
      "passkeys": [["id": "0", "createdOn": "Browser", "createdAt": 1_700_000_001.0]],
    ]
    await access.refreshAll([])
    XCTAssertEqual(access.accountEmail, "test@example.test")
    XCTAssertEqual(access.accountPasskeys.first?.createdOn, "Browser")
    XCTAssertEqual(access.passkeyCreatedOn, "MacBook")
    XCTAssertEqual(access.passkeyCreatedAt, Date(timeIntervalSince1970: 1_700_000_000))
    runtime.passkey = [:]
    runtime.accountDetails = [:]
    await access.refreshAll([])
    XCTAssertNil(access.accountEmail)
    XCTAssertTrue(access.accountPasskeys.isEmpty)
    XCTAssertFalse(access.accountDetailsLoaded)
    XCTAssertNil(access.passkeyCreatedOn)
    XCTAssertNil(access.passkeyCreatedAt)
    runtime.passkey = ["createdOn": "MacBook", "createdAt": 1_700_000_000.0]
    await access.refreshAll([])
    access.reset()
    XCTAssertNil(access.passkeyCreatedOn)
    XCTAssertNil(access.passkeyCreatedAt)
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
  var passkey: [String: Any] = [:]
  var accountDetails: [String: Any] = [:]
  var permissionDenied = false
  var accountRead: (() async -> Void)?
  var mintRead: (() async -> Void)?
  var spaceWarnings: [String] = []
  var invites: [[String: Any]] = []
  var spaceReads = 0
  func createCLILink(_ space: TonkSpace) async throws -> String {
    if permissionDenied { throw HarnessError.message("Permission denied") }
    return "unused"
  }
  func accountScript(_ script: String, arguments: [String: Any]) async throws -> [String: Any] {
    if permissionDenied { throw HarnessError.message("Permission denied") }
    if script == AccessModel.accountScript {
      await accountRead?()
      if accountFails { throw HarnessError.message("Offline") }
      return accountDetails.merging([
        "connections": [], "name": "Verifier", "root": "did:key:zVerifier", "passkey": passkey,
      ]) { _, value in value }
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
