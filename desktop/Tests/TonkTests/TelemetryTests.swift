import XCTest

@testable import Tonk

@MainActor
final class TelemetryTests: XCTestCase {
  private func telemetry() throws -> Telemetry {
    let defaults = try XCTUnwrap(UserDefaults(suiteName: "TelemetryTests-\(UUID().uuidString)"))
    // An unreachable endpoint: nothing here may reach PostHog.
    return Telemetry(
      environment: ["TONK_POSTHOG_KEY": "phc_test", "TONK_POSTHOG_ENDPOINT": "http://127.0.0.1:9"],
      defaults: defaults)
  }

  func testTurningSharingBackOnSchedulesTheNextBatch() throws {
    let telemetry = try telemetry()
    try XCTSkipUnless(telemetry.isAvailable, "Telemetry is unavailable against a local runtime.")
    telemetry.appOpened()
    XCTAssertTrue(telemetry.hasScheduledFlush)
    telemetry.setEnabled(false)
    XCTAssertFalse(telemetry.hasScheduledFlush)
    telemetry.setEnabled(true)
    telemetry.appOpened()
    XCTAssertTrue(telemetry.hasScheduledFlush)
  }

  func testTurningSharingOffQueuesNothing() throws {
    let telemetry = try telemetry()
    telemetry.setEnabled(false)
    telemetry.appOpened()
    XCTAssertFalse(telemetry.hasScheduledFlush)
  }
}
