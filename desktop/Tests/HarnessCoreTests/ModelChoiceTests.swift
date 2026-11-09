import XCTest

@testable import HarnessCore

final class ModelChoiceTests: XCTestCase {
  func testModelChoiceSeedsThenFollowsWhicheverSideChanged() {
    let claude = ModelChoice(provider: .claude, model: "opus")
    let local = ModelChoice(provider: .local, model: "fixture")
    XCTAssertEqual(ModelChoice.reconcile(local: local, remote: nil, synced: nil), .push(local))
    XCTAssertEqual(
      ModelChoice.reconcile(local: local, remote: .known(local), synced: nil), .agree(local))
    // Another desktop seeded the account first, or changed it since.
    XCTAssertEqual(
      ModelChoice.reconcile(local: local, remote: .known(claude), synced: nil), .adopt(claude))
    XCTAssertEqual(
      ModelChoice.reconcile(local: local, remote: .known(claude), synced: local), .adopt(claude))
    // This Mac changed while the account kept what both last agreed on.
    XCTAssertEqual(
      ModelChoice.reconcile(local: local, remote: .known(claude), synced: claude), .push(local))
    XCTAssertEqual(
      ModelChoice.reconcile(local: local, remote: .unknown, synced: claude), .ignore)
    XCTAssertEqual(
      ModelChoice.RemoteValue(provider: "someday", model: "x"), .unknown)
    XCTAssertEqual(
      ModelChoice.RemoteValue(provider: "claude", model: nil), .known(ModelChoice(provider: .claude)))
  }
}
