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
  func testEmailInvitationRetrySurvivesRestartAndDoesNotCreateAContact() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let runtime = ContactRuntime()
    let space = try TonkSpace.decodeCatalog(Data(#"[{"subject":"did:key:zShared","name":"shared"}]"#.utf8))[0]
    let access = AccessModel()
    access.prepare(directory: directory, runtime: runtime)
    await access.refreshAccount()
    runtime.sendResult = ["state": "unknown", "message": "Delivery could not be confirmed."]
    do {
      try await access.sendEmailInvitation(email: " Carol@Example.com ", space: space)
      XCTFail("An uncertain delivery must not show sent")
    } catch {}
    XCTAssertEqual(runtime.sends.count, 1)
    XCTAssertEqual(runtime.sends[0]["email"] as? String, "carol@example.com")
    let first = try XCTUnwrap(runtime.sends[0]["id"] as? String)
    XCTAssertEqual(first.count, 64)
    XCTAssertTrue(access.contacts.isEmpty)
    let reopened = AccessModel()
    reopened.prepare(directory: directory, runtime: runtime)
    await reopened.refreshAccount()
    runtime.sendResult = ["state": "sent"]
    try await reopened.sendEmailInvitation(email: "carol@example.com", space: space)
    XCTAssertEqual(runtime.sends[1]["id"] as? String, first)
    XCTAssertNotEqual(runtime.sends[0]["request"] as? String, runtime.sends[1]["request"] as? String)
    XCTAssertTrue(reopened.contacts.isEmpty)
    runtime.root = "did:key:zOtherOwner"
    await reopened.refreshAccount()
    try await reopened.sendEmailInvitation(email: "carol@example.com", space: space)
    XCTAssertNotEqual(runtime.sends[2]["id"] as? String, first)
    XCTAssertEqual(runtime.sends[2]["account"] as? String, runtime.root)
    try Data("damaged".utf8).write(to: directory.appendingPathComponent("email-invitations.json"))
    do {
      try await reopened.sendEmailInvitation(email: "carol@example.com", space: space)
      XCTFail("Damaged retry state must fail before sending")
    } catch {}
    XCTAssertEqual(runtime.sends.count, 3)
  }

  func testOnlyACompleteEmailIsLookedUpWhileTyping() {
    let model = ContactLookupModel { _ in .notFound }
    for partial in ["", "jack", "jack@", "jack@tonk", "jack@tonk.", "jack@tonk.x", "@tonk.xyz",
      "jack @tonk.xyz", "a@b@tonk.xyz", "did:key:z6Mk"]
    {
      model.input = partial
      XCTAssertFalse(model.isCompleteEmail, partial)
    }
    for complete in ["jack@tonk.xyz", " jack+bla@tonk.xyz ", "a@mail.example.co"] {
      model.input = complete
      XCTAssertTrue(model.isCompleteEmail, complete)
    }
  }

}

@MainActor
private final class ContactRuntime: SpaceAccessRuntime {
  var catalogBranch: String? = "main"
  var root = "did:key:zOwner"
  var sends: [[String: Any]] = []
  var sendResult: [String: Any] = ["state": "sent"]
  func createCLILink(_ space: TonkSpace) async throws -> String { "unused" }
  func accountScript(_ script: String, arguments: [String: Any]) async throws -> [String: Any] {
    if script == AccessModel.emailInvitationScript {
      sends.append(arguments)
      return sendResult
    }
    return ["connections": [], "name": "Owner", "root": root]
  }

}
