import AppKit
import SwiftUI

/// Use the same button style as the toolbar, with a pull-down menu below its anchor.
struct AnchoredMenuButton: View {
  var makeMenu: () -> NSMenu
  @State private var anchor = MenuAnchorView()

  var body: some View {
    Button {
      let menu = makeMenu()
      let point = NSPoint(
        x: anchor.bounds.maxX - menu.size.width,
        y: anchor.isFlipped ? anchor.bounds.maxY + 4 : anchor.bounds.minY - 4)
      menu.popUp(positioning: nil, at: point, in: anchor)
    } label: {
      Image(systemName: "ellipsis").frame(width: 20, height: 20)
    }
    .nativeControl(circular: true).controlSize(.large)
    .background(MenuAnchor(view: anchor))
    .accessibilityLabel("More options").help("Refresh, account and model settings")
  }
}

private struct MenuAnchor: NSViewRepresentable {
  let view: MenuAnchorView
  func makeNSView(context: Context) -> MenuAnchorView { view }
  func updateNSView(_ nsView: MenuAnchorView, context: Context) {}
}

private final class MenuAnchorView: NSView {
  override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

extension NSMenu {
  func addAction(
    _ title: String, symbol: String? = nil, key: String = "", enabled: Bool = true,
    action: @escaping () -> Void
  ) {
    let item = ClosureMenuItem(title: title, key: key, action: action)
    item.isEnabled = enabled
    if let symbol { item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil) }
    addItem(item)
  }
}

private final class ClosureMenuItem: NSMenuItem {
  let handler: () -> Void
  init(title: String, key: String, action: @escaping () -> Void) {
    handler = action
    super.init(title: title, action: #selector(invoke), keyEquivalent: key)
    target = self
  }
  required init(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }
  @objc private func invoke() { handler() }
}
