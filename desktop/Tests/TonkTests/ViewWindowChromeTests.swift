import AppKit
import XCTest

@testable import Tonk

@MainActor
final class ViewWindowChromeTests: XCTestCase {
  func testCustomControlsOperateTheirOwnWindow() {
    _ = NSApplication.shared
    let window = ActionWindow(
      contentRect: NSRect(x: 0, y: 0, width: 700, height: 500),
      styleMask: [.titled, .closable, .miniaturizable, .resizable],
      backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    defer { window.close() }
    let controls = ViewWindowControls.Controls()
    window.contentView?.addSubview(controls)
    let buttons = controls.subviews.compactMap { $0 as? NSButton }
    XCTAssertEqual(buttons.count, 3)
    buttons[0].performClick(nil)
    buttons[1].performClick(nil)
    buttons[2].performClick(nil)
    XCTAssertEqual(window.actions, ["close", "minimize", "fullscreen"])

    let configuration = ViewWindowConfiguration.Configuration()
    window.contentView?.addSubview(configuration)
    XCTAssertTrue(window.styleMask.contains(.fullSizeContentView))
    XCTAssertNil(window.toolbar)
    XCTAssertTrue(window.standardWindowButton(.closeButton)?.isHidden == true)
    XCTAssertFalse(buttons[0].isHidden, "Only the original titlebar buttons should be hidden")
  }

  func testHoverRegionDoesNotConsumeEmbeddedViewClicks() {
    var changes: [Bool] = []
    let region = ViewChromeHoverRegion.Region { changes.append($0) }
    region.frame = NSRect(x: 0, y: 0, width: 700, height: 52)
    region.updateTrackingAreas()
    XCTAssertNil(region.hitTest(NSPoint(x: 100, y: 20)))
    XCTAssertEqual(region.trackingAreas.count, 1)
    region.updateTrackingAreas()
    XCTAssertEqual(region.trackingAreas.count, 1, "Resizing must replace the tracking area")
    let event = NSEvent()
    region.mouseEntered(with: event)
    region.mouseExited(with: event)
    XCTAssertEqual(changes, [true, false])
  }
}

@MainActor
private final class ActionWindow: NSWindow {
  var actions: [String] = []
  override func performClose(_ sender: Any?) { actions.append("close") }
  override func performMiniaturize(_ sender: Any?) { actions.append("minimize") }
  override func toggleFullScreen(_ sender: Any?) { actions.append("fullscreen") }
}
