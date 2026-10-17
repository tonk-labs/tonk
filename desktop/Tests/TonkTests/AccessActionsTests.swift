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
  var accountRead: (() async -> Void)?
  var mintRead: (() async -> Void)?
  var spaceWarnings: [String] = []
  var invites: [[String: Any]] = []
  var spaceReads = 0
  func createCLILink(_ space: TonkSpace) async throws -> String { "unused" }
  func accountScript(_ script: String, arguments: [String: Any]) async throws -> [String: Any] {
    if script == AccessModel.accountScript {
      await accountRead?()
      if accountFails { throw HarnessError.message("Offline") }
      return ["connections": [], "name": "Verifier", "root": "did:key:zVerifier"]
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
