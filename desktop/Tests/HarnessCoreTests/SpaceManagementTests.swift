import XCTest

@testable import HarnessCore

final class SpaceManagementTests: XCTestCase {
  let root = "did:key:zRoot"

  private func space(_ fields: String) throws -> TonkSpace {
    try XCTUnwrap(
      TonkSpace.decodeCatalog(Data(#"[{"subject":"did:key:zSpace"\#(fields)}]"#.utf8)).first)
  }

  func testNamesAreTrimmedAndBounded() {
    XCTAssertEqual(SpaceName.validated("  Garden  "), "Garden")
    XCTAssertNil(SpaceName.validated("   "))
    XCTAssertNil(SpaceName.validated("a\nb"))
    XCTAssertNotNil(SpaceName.validated(String(repeating: "x", count: 80)))
    XCTAssertNil(SpaceName.validated(String(repeating: "x", count: 81)))
  }

  func testASpaceThisAccountHostsIsDeletedEverywhere() throws {
    let hosted = try space(#","provider":"did:key:zRoot","founded":true"#)
    XCTAssertEqual(hosted.removal(accountRoot: root), .deleteHosted)
    XCTAssertEqual(hosted.removal(accountRoot: root).verb, "Delete")
  }

  func testASpaceAnotherAccountHostsIsLeft() throws {
    let joined = try space(#","provider":"did:key:zOther""#)
    XCTAssertEqual(joined.removal(accountRoot: root), .leave)
    XCTAssertEqual(joined.removal(accountRoot: root).verb, "Leave")
  }

  func testAnUnhostedSpaceIsDeletedOnlyWhenFoundedHere() throws {
    XCTAssertEqual(try space(#","founded":true"#).removal(accountRoot: root), .deleteLocal)
    XCTAssertEqual(try space("").removal(accountRoot: root), .leave)
  }

  func testCatalogsWithoutOwnershipFieldsStillDecode() throws {
    let cached = try space(#","name":"Garden","icon":null,"color":null"#)
    XCTAssertNil(cached.provider)
    XCTAssertNil(cached.founded)
    XCTAssertEqual(cached.title, "Garden")
  }
}
