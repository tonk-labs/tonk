import Foundation
import XCTest

@testable import HarnessCore

final class TonkAuthorizationTests: XCTestCase {
  private let customRemote = try! RuntimeLocation.remote("remote.example")
  func testLocalRuntimeIsStrictlyIsolatedByOriginAndDataDirectory() {
    let local = RuntimeLocation.Deployment.local
    XCTAssertEqual(local.home.absoluteString, "http://127.0.0.1:4187")
    XCTAssertNotEqual(local.dataDirectory, customRemote.dataDirectory)
    XCTAssertNotEqual(local.dataDirectory, RuntimeLocation.Deployment.production.dataDirectory)
    XCTAssertTrue(
      RuntimeLocation.isEmbedded(local.home.appendingPathComponent("space/test"), deployment: local)
    )
    for value in [
      "http://127.0.0.1:4188", "http://localhost:4187", "http://127.0.0.1",
      "https://127.0.0.1:4187", "http://user@127.0.0.1:4187", "https://remote.example",
      "https://tonk.network",
    ] {
      XCTAssertFalse(RuntimeLocation.isEmbedded(URL(string: value)!, deployment: local), value)
    }
    for deployment: RuntimeLocation.Deployment in [.production, customRemote] {
      XCTAssertFalse(RuntimeLocation.isEmbedded(local.home, deployment: deployment))
    }
  }

  func testDeploymentsRejectEachOthersAuthorization() throws {
    XCTAssertNoThrow(
      try TonkAuthorization(
        data: payload(remote: "https://remote.example/ucan/"), deployment: customRemote))
    XCTAssertThrowsError(try TonkAuthorization(data: payload(), deployment: customRemote))
    XCTAssertThrowsError(
      try TonkAuthorization(
        data: payload(remote: "https://remote.example/ucan/"), deployment: .production))
    XCTAssertFalse(
      RuntimeLocation.isEmbedded(URL(string: "https://tonk.network")!, deployment: customRemote))
    XCTAssertFalse(
      RuntimeLocation.isEmbedded(URL(string: "https://remote.example")!, deployment: .production))
    XCTAssertNotEqual(
      customRemote.dataDirectory,
      RuntimeLocation.Deployment.production.dataDirectory)
  }
  private func payload(remote: String = "https://tonk.network/ucan/", delegation: String = "00aAFF")
    throws -> Data
  {
    try JSONSerialization.data(withJSONObject: [
      "delegationHex": delegation, "credentialId": "test-credential",
      "remote": remote, "attachmentId": "test-attachment",
    ])
  }

  func testAcceptsTonkCallbackWithTrailingSlash() throws {
    let remote = "https://tonk.network/ucan/"
    // Foundation's URL.path drops this slash; compare URLComponents instead.
    XCTAssertEqual(URL(string: remote)?.path, "/ucan")
    let grant = try TonkAuthorization(data: payload(remote: remote))
    XCTAssertEqual(grant.remote, remote)
    XCTAssertEqual(grant.delegation, "00aAFF")
    XCTAssertEqual(grant.credential, "test-credential")
    XCTAssertNoThrow(try TonkAuthorization(data: payload(remote: "https://tonk.network/ucan")))
  }

  func testRejectsOtherDestinationsAndMalformedGrantsWithoutEchoingSecrets() throws {
    for remote in [
      "http://tonk.network/ucan/", "https://evil.example/ucan/",
      "https://tonk.network.evil.example/ucan/", "https://user@tonk.network/ucan/",
      "https://tonk.network:444/ucan/", "https://tonk.network/other/",
      "https://tonk.network/ucan/?secret=private", "https://tonk.network/ucan/#private",
      "https://tonk.network/%75can/",
    ] {
      XCTAssertThrowsError(try TonkAuthorization(data: payload(remote: remote))) { error in
        XCTAssertFalse(error.localizedDescription.contains("private"))
      }
    }
    for delegation in ["", "abc", "secret", "ＦＦ"] {
      XCTAssertThrowsError(try TonkAuthorization(data: payload(delegation: delegation)))
    }
    XCTAssertThrowsError(try TonkAuthorization(data: Data("{}".utf8)))
    XCTAssertThrowsError(try TonkAuthorization(data: Data("private invalid JSON".utf8))) { error in
      XCTAssertFalse(error.localizedDescription.contains("private"))
    }
  }
}
