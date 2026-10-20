import AppKit
import SwiftUI

/// Native window buttons in the sidebar (or its transient top overlay).
/// These are AppKit controls, so accessibility and window actions stay native.
struct ViewWindowControls: NSViewRepresentable {
  func makeNSView(context: Context) -> Controls { Controls() }
  func updateNSView(_ view: Controls, context: Context) {}

  final class Controls: NSView {
    override var intrinsicContentSize: NSSize { NSSize(width: 56, height: 28) }
    override init(frame: NSRect) {
      super.init(frame: frame)
      let kinds: [NSWindow.ButtonType] = [.closeButton, .miniaturizeButton, .zoomButton]
      for (index, kind) in kinds.enumerated() {
        guard
          let button = NSWindow.standardWindowButton(
            kind, for: [.titled, .closable, .miniaturizable, .resizable])
        else { continue }
        button.target = self
        button.action =
          [#selector(closeWindow), #selector(minimizeWindow), #selector(zoomWindow)][index]
        button.setFrameOrigin(NSPoint(x: CGFloat(index) * 20, y: 7))
        addSubview(button)
      }
    }
    convenience init() { self.init(frame: .zero) }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    @objc private func closeWindow() { window?.performClose(nil) }
    @objc private func minimizeWindow() { window?.performMiniaturize(nil) }
    @objc private func zoomWindow() {
      if NSEvent.modifierFlags.contains(.option) {
        window?.performZoom(nil)
      } else {
        window?.toggleFullScreen(nil)
      }
    }
  }
}

/// A drag region behind custom chrome, without making embedded web content draggable.
struct ViewWindowDragRegion: NSViewRepresentable {
  func makeNSView(context: Context) -> Region { Region() }
  func updateNSView(_ view: Region, context: Context) {}
  final class Region: NSView {
    override func mouseDown(with event: NSEvent) {
      if event.clickCount == 2 {
        window?.performZoom(nil)
      } else {
        window?.performDrag(with: event)
      }
    }
  }
}

struct ViewWindowConfiguration: NSViewRepresentable {
  func makeNSView(context: Context) -> Configuration { Configuration() }
  func updateNSView(_ view: Configuration, context: Context) { view.configure() }
  final class Configuration: NSView {
    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      configure()
    }
    func configure() {
      guard let window else { return }
      window.titleVisibility = .hidden
      window.titlebarAppearsTransparent = true
      window.titlebarSeparatorStyle = .none
      window.styleMask.insert(.fullSizeContentView)
      window.toolbar = nil
      for kind in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
        window.standardWindowButton(kind)?.isHidden = true
      }
    }
  }
}

struct ViewChromeButton: View {
  let title: String
  let asset: String
  var action: () -> Void
  var body: some View {
    Button(action: action) { DesktopGlyph(name: asset, size: 15).frame(width: 28, height: 28) }
      .buttonStyle(.plain).background(Color(nsColor: .textBackgroundColor), in: Circle())
      .overlay(Circle().strokeBorder(.primary.opacity(0.07), lineWidth: 0.5))
      .help(title).accessibilityLabel(title)
  }
}

/// Tracks the top edge even when the controls are hidden, without intercepting web clicks.
struct ViewChromeHoverRegion: NSViewRepresentable {
  var onHover: (Bool) -> Void
  func makeNSView(context: Context) -> Region { Region(onHover: onHover) }
  func updateNSView(_ view: Region, context: Context) { view.onHover = onHover }

  final class Region: NSView {
    var onHover: (Bool) -> Void
    private var tracking: NSTrackingArea?
    private var monitor: Any?
    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      if let monitor { NSEvent.removeMonitor(monitor) }
      monitor = nil
      guard let window else { return }
      window.acceptsMouseMovedEvents = true
      // WKWebView has its own tracking areas. Observe the window's pointer events
      // as well so it cannot prevent the chrome from appearing above it.
      monitor = NSEvent.addLocalMonitorForEvents(matching: [.mouseMoved, .leftMouseDown]) {
        [weak self] event in
        guard let self, event.window === self.window, let content = self.window?.contentView
        else { return event }
        let point = content.convert(event.locationInWindow, from: nil)
        let atTop =
          content.isFlipped
          ? point.y <= content.bounds.minY + 52 : point.y >= content.bounds.maxY - 52
        self.onHover(atTop && content.bounds.contains(point))
        return event
      }
    }
    deinit { if let monitor { NSEvent.removeMonitor(monitor) } }
    init(onHover: @escaping (Bool) -> Void) {
      self.onHover = onHover
      super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
    override func hitTest(_ point: NSPoint) -> NSView? { nil }
    override func updateTrackingAreas() {
      super.updateTrackingAreas()
      if let tracking { removeTrackingArea(tracking) }
      let area = NSTrackingArea(
        rect: bounds, options: [.mouseEnteredAndExited, .activeAlways, .inVisibleRect],
        owner: self, userInfo: nil)
      addTrackingArea(area)
      tracking = area
    }
    override func mouseEntered(with event: NSEvent) { onHover(true) }
    override func mouseExited(with event: NSEvent) { onHover(false) }
  }
}
