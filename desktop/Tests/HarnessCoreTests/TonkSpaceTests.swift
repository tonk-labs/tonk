import Foundation
import XCTest

@testable import HarnessCore

final class TonkSpaceTests: XCTestCase {
  func testCatalogKeepsDuplicateNamesDistinctAndBuildsSafeRoutes() throws {
    let data = Data(
      """
      [{"subject":"did:key:z234","name":"Same"},
       {"subject":"did:key:z123","name":"Same"},
       {"subject":"did:key:z345","name":null}]
      """.utf8)
    let spaces = try TonkSpace.decodeCatalog(data)
    XCTAssertEqual(spaces.map(\.id), ["did:key:z123", "did:key:z234", "did:key:z345"])
    XCTAssertEqual(spaces.last?.title, "Untitled")
    XCTAssertEqual(spaces.first?.url.absoluteString, "https://tonk.network/space/did:key:z123")
  }
  func testRejectsMalformedOrDuplicateIdentity() {
    for json in [
      "[{\"subject\":\"did:key:z123/../../settings\"}]",
      "[{\"subject\":\"https://evil.example\"}]",
      "[{\"subject\":\"did:key:z123\"},{\"subject\":\"did:key:z123\"}]",
      "[{\"name\":\"Missing identity\"}]",
    ] { XCTAssertThrowsError(try TonkSpace.decodeCatalog(Data(json.utf8))) }
  }
}
