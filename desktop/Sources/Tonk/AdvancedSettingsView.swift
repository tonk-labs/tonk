import HarnessCore
import SwiftUI

struct AdvancedSettingsView: View {
  @Environment(\.dismiss) private var dismiss
  @State private var custom: String
  @State private var error: String?
  @State private var saved = false

  init() {
    let remote = RuntimeLocation.resolve(arguments: [], defaults: .standard)
    _custom = State(initialValue: remote.home.absoluteString)
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      Text("Advanced settings").font(.title2.weight(.semibold))
      HStack(spacing: 12) {
        TextField("Remote URL", text: $custom, prompt: Text("https://tonk.network"))
          .textFieldStyle(.roundedBorder).autocorrectionDisabled()
        Button("Reset") {
          custom = RuntimeLocation.Deployment.production.home.absoluteString
        }
        .help("Reset to tonk.network")
      }
      Text(
        "Each remote has its own account, spaces, and chats on this Mac. Quit and reopen Tonk to use the selected remote."
      )
      .font(.callout).foregroundStyle(.secondary)
      if ProcessInfo.processInfo.arguments.contains("--local-runtime") {
        Text(
          "This launch uses a command-line override. Open Tonk without that flag to use this setting."
        )
        .font(.caption).foregroundStyle(.secondary)
      }
      if let error { Text(error).foregroundStyle(.red) }
      if saved {
        Text("Remote saved. Quit and reopen Tonk to apply it.").foregroundStyle(.secondary)
      }
      HStack {
        Spacer()
        Button(saved ? "Done" : "Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
        Button("Save") {
          do {
            let remote = try RuntimeLocation.remote(custom)
            UserDefaults.standard.set(
              remote.home.absoluteString, forKey: RuntimeLocation.preferenceKey)
            error = nil
            saved = true
          } catch { self.error = error.localizedDescription }
        }.nativeControl(prominent: true).keyboardShortcut(.defaultAction).disabled(saved)
      }
    }
    .padding(24).frame(width: 540).nativeControl()
    .onChange(of: custom) { _, _ in
      saved = false
      error = nil
    }
  }
}
