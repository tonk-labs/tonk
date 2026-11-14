import HarnessCore
import SwiftUI

/// Names a new space, or renames one. Stays open while the worker confirms,
/// so a failure is shown where the name was typed.
struct SpaceNameSheet: View {
  let title: String
  let confirm: String
  let save: (String) async throws -> Void
  @State var name: String
  @State private var saving = false
  @State private var error: String?
  @FocusState private var focused: Bool
  @Environment(\.dismiss) private var dismiss

  init(
    title: String, confirm: String, name: String = "", save: @escaping (String) async throws -> Void
  ) {
    self.title = title
    self.confirm = confirm
    self.save = save
    _name = State(initialValue: name)
  }

  private var canSave: Bool { !saving && SpaceName.validated(name) != nil }

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text(title).font(.headline)
      VStack(alignment: .leading, spacing: 6) {
        Text("Name").font(.caption).foregroundStyle(.secondary)
        TextField("Name", text: $name).textFieldStyle(.roundedBorder)
          .accessibilityLabel("Space name").focused($focused).onSubmit(submit)
          .disabled(saving)
      }
      if let error { Text(error).foregroundStyle(.red) }
      HStack {
        if saving { ProgressView().controlSize(.small) }
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction).disabled(saving)
        Button(confirm, action: submit).keyboardShortcut(.defaultAction).disabled(!canSave)
      }
    }
    .padding(22).frame(width: 400)
    .interactiveDismissDisabled(saving)
    .onAppear { focused = true }
  }

  private func submit() {
    guard canSave else { return }
    saving = true
    error = nil
    Task {
      defer { saving = false }
      do {
        try await save(name)
        dismiss()
      } catch { self.error = AccessModel.message(error) }
    }
  }
}
