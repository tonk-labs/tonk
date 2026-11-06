import XCTest

@testable import Tonk

final class AppUpdaterTests: XCTestCase {
  private let published: [String: Any] = [
    "SUFeedURL": "https://example.test/appcast.xml", "SUPublicEDKey": "key",
  ]

  func testPublishedBuildsCheckForUpdates() {
    XCTAssertTrue(AppUpdater.isEnabled(info: published, arguments: ["Tonk"]))
  }

  func testBuildsWithoutAFeedOrKeyNeverUpdate() {
    XCTAssertFalse(AppUpdater.isEnabled(info: [:], arguments: ["Tonk"]))
    XCTAssertFalse(
      AppUpdater.isEnabled(info: ["SUFeedURL": published["SUFeedURL"]!], arguments: ["Tonk"]))
    XCTAssertFalse(
      AppUpdater.isEnabled(
        info: ["SUFeedURL": published["SUFeedURL"]!, "SUPublicEDKey": " "], arguments: ["Tonk"]))
  }

  func testTestCopiesNeverUpdate() {
    for flag in ["--data-dir", "--local-runtime"] {
      XCTAssertFalse(AppUpdater.isEnabled(info: published, arguments: ["Tonk", flag, "/tmp/x"]))
    }
  }
}
