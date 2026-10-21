import Foundation
import XCTest

@testable import HarnessCore
@testable import Tonk

@MainActor
final class ContactLookupTests: XCTestCase {
  private func identity(_ email: String = "alice@example.com", root: String = "did:key:zAlice")
    -> EmailContactIdentity
  {
    EmailContactIdentity(
      email: email, rootDid: root, directory: URL(string: "https://tonk.example")!,
      checkedAt: Date(timeIntervalSince1970: 1000))
  }

  func testEditAndCancellationDiscardLateResponses() async throws {
    var continuations: [CheckedContinuation<EmailContactResult, Error>] = []
    let model = ContactLookupModel { _ in
      try await withCheckedThrowingContinuation { continuations.append($0) }
    }
    model.input = "alice@example.com"
    let first = Task { await model.lookup() }
    while continuations.count < 1 { await Task.yield() }
    // A second click must not start a duplicate request.
    await model.lookup()
    XCTAssertEqual(continuations.count, 1)
    model.input = "bob@example.com"
    XCTAssertNil(model.did)
    let second = Task { await model.lookup() }
    while continuations.count < 2 { await Task.yield() }
    continuations[1].resume(returning: .found(identity("bob@example.com", root: "did:key:zBob")))
    await second.value
    continuations[0].resume(returning: .found(identity()))
    await first.value
    XCTAssertEqual(model.did, "did:key:zBob")
    model.input = "alice@example.com"
    let cancelled = Task { await model.lookup() }
    while continuations.count < 3 { await Task.yield() }
    // This is also the invalidation the sheet uses on account switch or close.
    model.invalidate()
    cancelled.cancel()
    continuations[2].resume(returning: .found(identity()))
    await cancelled.value
    XCTAssertNil(model.did)
    XCTAssertFalse(model.isLoading)
  }

  func testFailureCannotKeepPreviousKeyAndManualEntryNeedsNoLookup() async {
    var response: EmailContactResult = .found(identity())
    var shouldFail = false
    var calls = 0
    let model = ContactLookupModel { _ in
      calls += 1
      if shouldFail { throw EmailContactError.unavailable }
      return response
    }
    model.input = "alice@example.com"
    await model.lookup()
    XCTAssertEqual(model.did, "did:key:zAlice")
    for unavailable in [EmailContactResult.pending, .notFound, .suspended] {
      response = unavailable
      await model.lookup()
      XCTAssertNil(model.did)
      XCTAssertEqual(model.result, unavailable)
    }
    shouldFail = true
    await model.lookup()
    XCTAssertNil(model.did)
    XCTAssertNotNil(model.error)
    XCTAssertFalse(model.isLoading)
    let beforeManual = calls
    model.input = " did:key:zManual "
    await model.lookup()
    XCTAssertEqual(model.did, "did:key:zManual")
    XCTAssertNil(model.identity)
    XCTAssertEqual(calls, beforeManual)
  }

  func testExistingAddressBookRoundTripsLookupAndStaysAccountScoped() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let runtime = ContactRuntime()
    let legacy = DesktopContact(id: "did:key:zLegacy", name: "Legacy", labels: ["Team"])
    // Actual pre-change shape: no emailIdentity property.
    try Data(
      #"{"did:key:zOwner":[{"id":"did:key:zLegacy","name":"Legacy","labels":["Team"]}]}"#.utf8
    )
    .write(to: directory.appendingPathComponent("address-book.json"))
    let access = AccessModel()
    access.prepare(directory: directory, runtime: runtime)
    await access.refreshAccount()
    XCTAssertEqual(access.contacts, [legacy])
    var contact = DesktopContact(
      id: "did:key:zAlice", name: "Alice", labels: ["Family"], emailIdentity: identity())
    try access.saveContact(contact)
    contact.labels.append("Work")
    try access.saveContact(contact)
    let reopened = AccessModel()
    reopened.prepare(directory: directory, runtime: runtime)
    await reopened.refreshAccount()
    XCTAssertEqual(reopened.contacts.first(where: { $0.id == contact.id }), contact)
    XCTAssertEqual(reopened.contacts.first(where: { $0.id == legacy.id }), legacy)
    var mismatched = contact
    mismatched.id = "did:key:zOther"
    XCTAssertThrowsError(try reopened.saveContact(mismatched))
    reopened.reset()
    XCTAssertThrowsError(try reopened.saveContact(contact))
    runtime.root = "did:key:zSecondOwner"
    await reopened.refreshAccount()
    XCTAssertTrue(reopened.contacts.isEmpty)
    try reopened.saveContact(DesktopContact(id: "did:key:zSecond", name: "Second", labels: []))
    reopened.reset()
    runtime.root = "did:key:zOwner"
    await reopened.refreshAccount()
    XCTAssertEqual(reopened.contacts.count, 2)
    XCTAssertEqual(reopened.contacts.first(where: { $0.id == contact.id }), contact)
  }
}

@MainActor
private final class ContactRuntime: SpaceAccessRuntime {
  var catalogBranch: String? = "main"
  var root = "did:key:zOwner"
  func createCLILink(_ space: TonkSpace) async throws -> String { "unused" }
  func accountScript(_ script: String, arguments: [String: Any]) async throws -> [String: Any] {
    ["connections": [], "name": "Owner", "root": root]
  }
}
