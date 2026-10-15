import Foundation
import XCTest

@testable import HarnessCore

final class RuntimeLocationTests: XCTestCase {
  func testNormalizesKnownAndCustomRemotes() throws {
    XCTAssertEqual(try RuntimeLocation.remote(" TONK.NETWORK/ \n"), .production)
    let remote = try RuntimeLocation.remote("https://Other.example:8443/")
    XCTAssertEqual(remote.home.absoluteString, "https://other.example:8443")
    XCTAssertTrue(
      RuntimeLocation.isEmbedded(
        remote.home.appendingPathComponent("space/test"), deployment: remote))
    XCTAssertFalse(
      RuntimeLocation.isEmbedded(URL(string: "https://other.example")!, deployment: remote))
    XCTAssertFalse(
      RuntimeLocation.isEmbedded(
        URL(string: "https://user@other.example:8443")!, deployment: remote))
  }

  func testRejectsInvalidRemoteOrigins() {
    for value in [
      "", "http://example.com", "https://", "not a host", "https://example.com/path",
      "https://user@example.com", "https://example.com?query", "https://example.com#fragment",
      "https://example.com:0", "https://example.com:65536",
    ] {
      XCTAssertThrowsError(try RuntimeLocation.remote(value), value)
    }
  }

  func testSelectionPersistsWithExplicitLaunchOverrides() throws {
    let name = "RuntimeLocationTests-" + UUID().uuidString
    let defaults = UserDefaults(suiteName: name)!
    defer { defaults.removePersistentDomain(forName: name) }
    XCTAssertEqual(RuntimeLocation.resolve(arguments: [], defaults: defaults), .production)
    let custom = try RuntimeLocation.remote("other.example")
    defaults.set(custom.home.absoluteString, forKey: RuntimeLocation.preferenceKey)
    XCTAssertEqual(
      RuntimeLocation.resolve(arguments: [], defaults: UserDefaults(suiteName: name)!), custom)
    XCTAssertEqual(
      RuntimeLocation.resolve(arguments: ["--local-runtime"], defaults: defaults), .local)
  }

  func testRemoteDataAndAuthorizationAreIsolated() throws {
    let deployments: [RuntimeLocation.Deployment] = [
      .production, .local,
      try RuntimeLocation.remote("one.example"), try RuntimeLocation.remote("two.example"),
    ]
    XCTAssertEqual(Set(deployments.map(\.dataDirectory)).count, deployments.count)
    XCTAssertEqual(Set(deployments.compactMap(\.webDataIdentifier)).count, deployments.count - 1)
    XCTAssertEqual(
      try RuntimeLocation.remote("one.example").webDataIdentifier,
      try RuntimeLocation.remote("https://ONE.example:443/").webDataIdentifier)
    for source in deployments {
      let data = try JSONSerialization.data(withJSONObject: [
        "delegationHex": "00aaff", "credentialId": "test", "attachmentId": "test",
        "remote": source.home.absoluteString + "/ucan/",
      ])
      for target in deployments {
        if source == target {
          XCTAssertNoThrow(try TonkAuthorization(data: data, deployment: target))
        } else {
          XCTAssertThrowsError(try TonkAuthorization(data: data, deployment: target))
        }
      }
    }
  }
}
