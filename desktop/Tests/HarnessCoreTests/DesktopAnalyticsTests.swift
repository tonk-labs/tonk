import XCTest

@testable import HarnessCore

final class DesktopAnalyticsTests: XCTestCase {
  let context = DesktopAnalytics.Context(version: "0.1.0", arch: "aarch64")
  let when = Date(timeIntervalSince1970: 1_800_000_000)

  func testDistinctIDMatchesTheCLIAndWebHash() {
    // sha256("did:key:z6MkExample"), the same value `tonk_analytics::distinct_id` produces.
    XCTAssertEqual(
      DesktopAnalytics.distinctID(profileDID: "did:key:z6MkExample"),
      "tonk:beac80774be09b6217bf4f6c2c3e346193582423d50ea8f238669dd81373c853")
  }

  func testEnvironmentOptOutFollowsTheCLI() {
    XCTAssertFalse(DesktopAnalytics.environmentOptsOut([:]))
    XCTAssertFalse(DesktopAnalytics.environmentOptsOut(["DO_NOT_TRACK": "0"]))
    XCTAssertFalse(DesktopAnalytics.environmentOptsOut(["DO_NOT_TRACK": ""]))
    XCTAssertTrue(DesktopAnalytics.environmentOptsOut(["DO_NOT_TRACK": "1"]))
    XCTAssertTrue(DesktopAnalytics.environmentOptsOut(["TONK_TELEMETRY": "0"]))
    XCTAssertTrue(DesktopAnalytics.environmentOptsOut(["TONK_TELEMETRY": "off"]))
    XCTAssertTrue(DesktopAnalytics.environmentOptsOut(["TONK_TELEMETRY": "false"]))
    XCTAssertFalse(DesktopAnalytics.environmentOptsOut(["TONK_TELEMETRY": "1"]))
  }

  func testABuildWithoutAKeySendsNothing() {
    XCTAssertNil(DesktopAnalytics.apiKey(environment: [:], bundled: nil))
    XCTAssertNil(DesktopAnalytics.apiKey(environment: [:], bundled: ""))
    XCTAssertEqual(DesktopAnalytics.apiKey(environment: [:], bundled: "phc_baked"), "phc_baked")
    XCTAssertEqual(
      DesktopAnalytics.apiKey(environment: ["TONK_POSTHOG_KEY": "phc_env"], bundled: "phc_baked"),
      "phc_env")
  }

  func testHostDefaultsToTheEUCloudAndHonoursOverrides() {
    XCTAssertEqual(
      DesktopAnalytics.host(environment: [:], bundled: nil).absoluteString,
      "https://eu.i.posthog.com")
    XCTAssertEqual(
      DesktopAnalytics.host(
        environment: ["TONK_POSTHOG_ENDPOINT": "http://127.0.0.1:9"],
        bundled: "https://us.i.posthog.com"
      ).absoluteString, "http://127.0.0.1:9")
  }

  func testAProductEventCarriesOnlyTheClosedSchemaAndContext() {
    let started = DesktopAnalytics.started(
      .space, .publishSpace, stage: .intent, surface: .hub, trigger: .user, attemptID: "a1",
      context: context, at: when)
    XCTAssertEqual(started.name, "product_event")
    XCTAssertEqual(
      Set(started.properties.keys),
      [
        "schema_version", "journey", "action", "phase", "stage", "surface", "trigger",
        "attempt_id", "$lib", "environment", "version", "os", "arch",
      ])
    XCTAssertEqual(started.properties["action"], .string("publish_space"))
    XCTAssertEqual(started.properties["environment"], .string("desktop"))
    XCTAssertEqual(started.timestamp, "2027-01-15T08:00:00.000Z")
  }

  func testTerminalEventsKeepFailureKindsForFailuresOnlyAndCapDuration() {
    let success = DesktopAnalytics.finished(
      .workspace, .sendMessage, stage: .complete, surface: .workspace, trigger: .user,
      attemptID: "a2", durationMs: 9_999_999, result: .success, failureKind: .network,
      context: context, at: when)
    XCTAssertEqual(success.properties["result"], .string("success"))
    XCTAssertNil(success.properties["failure_kind"])
    XCTAssertEqual(success.properties["duration_ms"], .number(600_000))

    let failure = DesktopAnalytics.finished(
      .workspace, .sendMessage, stage: .worker, surface: .workspace, trigger: .user,
      attemptID: "a3", durationMs: -5, result: .retryableFailure, context: context, at: when)
    XCTAssertEqual(failure.properties["failure_kind"], .string("unknown"))
    XCTAssertEqual(failure.properties["duration_ms"], .number(0))
  }

  func testTheBatchAttachesIdentityAtSendTime() throws {
    let event = DesktopAnalytics.appOpened(context: context, at: when)
    let body = DesktopAnalytics.batch([event], apiKey: "phc_test", distinctID: "tonk:abc")
    XCTAssertEqual(body["api_key"], .string("phc_test"))
    guard case .array(let items) = body["batch"], let item = items.first else {
      return XCTFail("batch is not an array")
    }
    XCTAssertEqual(item["event"], .string("app_opened"))
    XCTAssertEqual(item["distinct_id"], .string("tonk:abc"))
    XCTAssertEqual(item["properties"]["$lib"], .string("tonk-desktop"))
  }

  func testAttemptIDsAreOpaqueAndDistinct() {
    let first = DesktopAnalytics.attemptID()
    XCTAssertEqual(first.count, 32)
    XCTAssertNotEqual(first, DesktopAnalytics.attemptID())
  }
}
