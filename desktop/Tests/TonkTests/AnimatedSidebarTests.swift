import AppKit
import SwiftUI
import XCTest

@testable import Tonk

@MainActor
final class AnimatedSidebarTests: XCTestCase {
  func testRepresentedDividerReceivesMouseEvents() {
    _ = NSApplication.shared
    let host = NSHostingController(
      rootView: AnimatedSidebar(
        isVisible: true, reduceMotion: false, sidebar: Color.white, detail: Color.gray
      )
      .frame(width: 1180, height: 780))
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1180, height: 780),
      styleMask: [.titled, .resizable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentViewController = host
    defer { window.close() }
    window.setContentSize(NSSize(width: 1180, height: 780))
    host.view.layoutSubtreeIfNeeded()
    func findSplit(_ view: NSView) -> NSSplitView? {
      if let split = view as? NSSplitView { return split }
      return view.subviews.compactMap { findSplit($0) }.first
    }
    guard let split = findSplit(host.view) else { return XCTFail("Missing split view") }
    let point = NSPoint(x: split.subviews[0].frame.maxX + split.dividerThickness / 2, y: 100)
    let hit = host.view.hitTest(host.view.convert(point, from: split))
    XCTAssertTrue(hit === split, "Divider hit went to \(String(describing: hit))")
  }

  func testCollapsePreservesControllersWidthAndWindowSize() {
    _ = NSApplication.shared
    let controller = SidebarSplitController(
      sidebar: Color.white, detail: Color.gray, isVisible: true)
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1180, height: 780),
      styleMask: [.titled, .resizable], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentViewController = controller
    window.setContentSize(NSSize(width: 1180, height: 780))
    defer { window.close() }
    controller.view.layoutSubtreeIfNeeded()
    controller.splitView.setPosition(420, ofDividerAt: 0)
    controller.view.layoutSubtreeIfNeeded()
    XCTAssertLessThan(
      controller.splitViewItems[0].holdingPriority.rawValue,
      NSLayoutConstraint.Priority.dragThatCannotResizeWindow.rawValue)
    let divider = NSRect(x: 420, y: 0, width: 1, height: 780)
    let hitRect = controller.splitView(
      controller.splitView, effectiveRect: divider.insetBy(dx: -3, dy: 0),
      forDrawnRect: divider, ofDividerAt: 0)
    XCTAssertFalse(hitRect.isEmpty, "The visible divider must accept mouse drags")
    window.setContentSize(NSSize(width: 1600, height: 780))
    controller.view.layoutSubtreeIfNeeded()
    controller.setMaximumSidebarWidth(800)
    controller.splitView.setPosition(1000, ofDividerAt: 0)
    controller.view.layoutSubtreeIfNeeded()
    XCTAssertEqual(controller.sidebarHost.view.frame.width, 800, accuracy: 1)
    controller.splitView.setPosition(420, ofDividerAt: 0)
    controller.view.layoutSubtreeIfNeeded()
    let sidebar = controller.sidebarHost
    let detail = controller.detailHost
    let width = sidebar.view.frame.width
    let frame = window.frame

    controller.setSidebarVisible(false, animated: false)
    controller.view.layoutSubtreeIfNeeded()
    XCTAssertTrue(controller.splitViewItems[0].isCollapsed)
    XCTAssertEqual(detail.view.frame.width, controller.view.bounds.width, accuracy: 1)
    controller.setSidebarVisible(true, animated: false)
    controller.view.layoutSubtreeIfNeeded()
    XCTAssertFalse(controller.splitViewItems[0].isCollapsed)
    XCTAssertTrue(controller.sidebarHost === sidebar)
    XCTAssertTrue(controller.detailHost === detail)
    XCTAssertEqual(sidebar.view.frame.width, width, accuracy: 1)
    XCTAssertEqual(window.frame, frame)
  }

  func testRapidAnimatedReversalFinishesAtLatestVisibility() async throws {
    _ = NSApplication.shared
    let controller = SidebarSplitController(
      sidebar: Color.white, detail: Color.gray, isVisible: false)
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1180, height: 780),
      styleMask: [.titled], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.contentViewController = controller
    window.setContentSize(NSSize(width: 1180, height: 780))
    defer { window.close() }
    controller.view.layoutSubtreeIfNeeded()
    var settled: [Bool] = []
    let opened = expectation(description: "Opening animation completed")
    let closed = expectation(description: "Closing animation completed")
    controller.onVisibilitySettled = {
      settled.append($0)
      if $0 { opened.fulfill() } else { closed.fulfill() }
    }
    controller.setSidebarVisible(true, animated: true)
    controller.setSidebarVisible(false, animated: true)
    controller.setSidebarVisible(true, animated: true)
    XCTAssertTrue(settled.isEmpty, "Focus must wait for the split-view animation")
    await fulfillment(of: [opened], timeout: 3)
    controller.view.layoutSubtreeIfNeeded()
    XCTAssertEqual(settled, [true])
    XCTAssertFalse(controller.splitViewItems[0].isCollapsed)
    XCTAssertGreaterThanOrEqual(controller.sidebarHost.view.frame.width, 340)
    XCTAssertGreaterThanOrEqual(controller.detailHost.view.frame.width, 380)
    controller.setSidebarVisible(false, animated: true)
    controller.setSidebarVisible(true, animated: true)
    controller.setSidebarVisible(false, animated: true)
    await fulfillment(of: [closed], timeout: 3)
    controller.view.layoutSubtreeIfNeeded()
    XCTAssertTrue(controller.splitViewItems[0].isCollapsed)
    XCTAssertEqual(settled, [true, false])
    XCTAssertEqual(
      controller.detailHost.view.frame.width, controller.view.bounds.width, accuracy: 1)
  }
}
