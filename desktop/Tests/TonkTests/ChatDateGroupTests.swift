import Foundation
import XCTest

@testable import Tonk

final class ChatDateGroupTests: XCTestCase {
  private var calendar: Calendar {
    var calendar = Calendar(identifier: .gregorian)
    calendar.timeZone = TimeZone(identifier: "UTC")!
    return calendar
  }
  /// Monday 26 October 2026, 15:00 UTC.
  private let now = Date(timeIntervalSince1970: 1_793_026_800)

  private func group(hoursAgo: Double) -> ChatDateGroup {
    ChatDateGroup(now.addingTimeInterval(-hoursAgo * 3600), now: now, calendar: calendar)
  }

  func testItGroupsChatsByDay() {
    XCTAssertEqual(group(hoursAgo: 0.5), .today)
    XCTAssertEqual(group(hoursAgo: 20), .yesterday)
    XCTAssertEqual(group(hoursAgo: 24 * 4), .previousWeek)
    XCTAssertEqual(group(hoursAgo: 24 * 30), .older)
  }

  func testItShowsMinutesAndHoursForToday() {
    XCTAssertEqual(ChatDateGroup.today.time(now.addingTimeInterval(-20), now: now), "now")
    XCTAssertEqual(ChatDateGroup.today.time(now.addingTimeInterval(-120), now: now), "2m")
    XCTAssertEqual(ChatDateGroup.today.time(now.addingTimeInterval(-3 * 3600), now: now), "3h")
  }
}
