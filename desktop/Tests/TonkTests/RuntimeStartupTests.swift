import XCTest

@testable import Tonk

@MainActor
final class RuntimeStartupTests: XCTestCase {
  func testStartupRemainsUnknownUntilValidAccountResponse() throws {
    let runtime = RuntimeModel()
    XCTAssertFalse(runtime.accountStatusKnown)
    XCTAssertFalse(runtime.accountConnected)
    XCTAssertThrowsError(try runtime.applyAccountStatus([:]))
    XCTAssertThrowsError(try runtime.applyAccountStatus(["status": "unexpected"]))
    XCTAssertFalse(runtime.accountStatusKnown)
    try runtime.applyAccountStatus(["status": "registered"])
    XCTAssertTrue(runtime.accountStatusKnown)
    XCTAssertTrue(runtime.accountConnected)
  }

  func testKnownSignedOutStatesAllowOnboarding() throws {
    for status in ["rootMissing", "unregistered"] {
      let runtime = RuntimeModel()
      try runtime.applyAccountStatus(["status": status])
      XCTAssertTrue(runtime.accountStatusKnown)
      XCTAssertFalse(runtime.accountConnected)
    }
  }
}
