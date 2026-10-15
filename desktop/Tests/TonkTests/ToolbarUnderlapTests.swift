import AppKit
import SwiftUI
import XCTest

@testable import Tonk

@MainActor
final class ToolbarUnderlapTests: XCTestCase {
  func testTranslucentToolbarKeepsPageBelowToolbar() async throws {
    try await checkGeometry(underlap: true)
  }

  func testOpaqueFallbackKeepsWebViewBelowToolbar() async throws {
    try await checkGeometry(underlap: false)
  }

  private func checkGeometry(underlap: Bool) async throws {
    guard #available(macOS 26.0, *) else { throw XCTSkip("Requires WebKit content insets") }
    _ = NSApplication.shared
    let runtime = RuntimeModel()
    let host = NSHostingController(
      rootView: GeometryReader { _ in
        AnimatedSidebar(
          isVisible: false, reduceMotion: true, sidebar: Color.gray,
          detail: RuntimeView(model: runtime))
      }.modifier(RuntimeToolbarBackground(enabled: underlap)))
    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 1000, height: 700),
      styleMask: [.titled, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
    window.isReleasedWhenClosed = false
    window.toolbar = NSToolbar(identifier: "Underlap fixture")
    window.toolbarStyle = .unified
    window.contentViewController = host
    window.setContentSize(NSSize(width: 1000, height: 700))
    defer { window.close() }
    for size in [NSSize(width: 1000, height: 700), NSSize(width: 850, height: 580)] {
      window.setContentSize(size)
      host.view.layoutSubtreeIfNeeded()
      try await Task.sleep(for: .milliseconds(100))
      host.view.layoutSubtreeIfNeeded()
      let webFrame = runtime.webView.convert(runtime.webView.bounds, to: nil)
      XCTAssertEqual(webFrame.maxY, window.contentLayoutRect.maxY, accuracy: 1)
      XCTAssertEqual(runtime.webView.obscuredContentInsets.top, 0)
    }
  }
}
