import AppKit
import SwiftUI

/// AppKit owns toolbar field focus; SwiftUI supplies the shared glass surface.
struct GlassToolbarSearch: View {
  @Binding var text: String
  let focusRequest: Int

  var body: some View {
    HStack(spacing: 8) {
      Image(systemName: "magnifyingglass").foregroundStyle(.secondary).accessibilityHidden(true)
      ToolbarSearchEditor(text: $text, focusRequest: focusRequest)
      if !text.isEmpty {
        Button("Clear search", systemImage: "xmark.circle.fill") { text = "" }
          .labelStyle(.iconOnly).buttonStyle(.plain)
      }
    }
    .padding(.horizontal, 12).frame(height: 34).controlSurface(radius: 17)
  }
}

private struct ToolbarSearchEditor: NSViewRepresentable {
  @Binding var text: String
  let focusRequest: Int

  func makeCoordinator() -> Coordinator { Coordinator(self) }

  func makeNSView(context: Context) -> NSTextField {
    let field = NSTextField()
    field.isBezeled = false
    field.drawsBackground = false
    field.focusRingType = .none
    field.font = .systemFont(ofSize: NSFont.systemFontSize)
    field.placeholderString = "Find a space"
    field.setAccessibilityLabel("Find a space")
    field.delegate = context.coordinator
    field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    return field
  }

  func updateNSView(_ field: NSTextField, context: Context) {
    context.coordinator.parent = self
    if field.stringValue != text { field.stringValue = text }
    guard context.coordinator.lastFocusRequest != focusRequest else { return }
    context.coordinator.lastFocusRequest = focusRequest
    field.window?.makeFirstResponder(field)
    field.selectText(nil)
  }

  final class Coordinator: NSObject, NSTextFieldDelegate {
    var parent: ToolbarSearchEditor
    var lastFocusRequest: Int

    init(_ parent: ToolbarSearchEditor) {
      self.parent = parent
      lastFocusRequest = parent.focusRequest
    }

    func controlTextDidChange(_ notification: Notification) {
      guard let field = notification.object as? NSTextField else { return }
      parent.text = field.stringValue
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy command: Selector) -> Bool
    {
      guard command == #selector(NSResponder.cancelOperation(_:)) else { return false }
      if parent.text.isEmpty {
        control.window?.makeFirstResponder(nil)
      } else {
        parent.text = ""
        control.stringValue = ""
      }
      return true
    }
  }
}
