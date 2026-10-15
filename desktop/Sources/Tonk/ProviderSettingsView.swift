import AppKit
import HarnessCore
import SwiftUI

struct ProviderSettingsView: View {
  @ObservedObject var model: HarnessModel
  @Environment(\.dismiss) private var dismiss
  @State private var selected: ModelProvider
  @State private var connection: ModelConnection
  @State private var codexExecutable: String?
  @State private var key = ""
  @State private var saving = false
  @State private var startedSignIn = false
  @State private var showAdvancedSettings = false
  @State private var error: String?

  init(model: HarnessModel) {
    self.model = model
    _selected = State(initialValue: model.provider)
    _connection = State(initialValue: model.connection)
    _codexExecutable = State(initialValue: model.saved.codexExecutable)
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 20) {
      Text("Settings").font(.title2.weight(.semibold))
      VStack(alignment: .leading, spacing: 12) {
        HStack {
          Text("Provider")
          Spacer()
          Menu {
            Picker("Provider", selection: $selected) {
              ForEach(ModelProvider.allCases) { provider in Text(provider.title).tag(provider) }
            }.pickerStyle(.inline)
          } label: {
            HStack(spacing: 8) {
              Text(selected.title)
              Image(systemName: "chevron.down").font(.caption.weight(.semibold))
            }
          }.modifier(SettingsMenuStyle())
            .disabled(model.loginPending || model.connecting)
        }
        if selected == .chatGPT {
          if model.codexDiscoveryFailed {
            VStack(alignment: .leading, spacing: 8) {
              HStack {
                Text("Codex CLI")
                Spacer()
                if codexExecutable != nil {
                  Button("Use automatic discovery") { codexExecutable = nil }
                }
                Button("Choose executable…") { chooseCodexExecutable() }
              }
              Text(
                codexExecutable ?? "Could not find Codex automatically. Choose your installed CLI."
              )
              .font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
              .lineLimit(2).truncationMode(.middle)
            }
          }
          if model.provider == .chatGPT && model.signedIn {
            if model.subscriptionModelsLoading {
              ProgressView("Loading models…").controlSize(.small)
            } else {
              HStack {
                Text("Model")
                Spacer()
                Menu {
                  Picker("Model", selection: $connection.model) {
                    Text("Default from ChatGPT").tag("")
                    if !connection.model.isEmpty
                      && !model.subscriptionModels.contains(where: { $0.id == connection.model })
                    {
                      Text("Unavailable: \(connection.model)").tag(connection.model).disabled(true)
                    }
                    ForEach(model.subscriptionModels) { item in Text(item.name).tag(item.id) }
                  }.pickerStyle(.inline)
                } label: {
                  HStack(spacing: 8) {
                    Text(
                      connection.model.isEmpty
                        ? "Default from ChatGPT"
                        : model.subscriptionModels.first(where: { $0.id == connection.model })?.name
                          ?? "Unavailable: \(connection.model)")
                    Image(systemName: "chevron.down").font(.caption.weight(.semibold))
                  }
                }.modifier(SettingsMenuStyle())
              }
              if let error = model.modelCatalogError {
                Text(error).font(.caption).foregroundStyle(.secondary)
              }
            }
          } else {
            subscriptionSignIn
          }
        } else if selected == .claude {
          if model.provider == .claude && model.signedIn {
            if model.claudeModelsLoading {
              ProgressView("Loading models…").controlSize(.small)
            } else {
              HStack {
                Text("Model")
                Spacer()
                Menu {
                  Picker(
                    "Model",
                    selection: Binding(
                      get: { connection.model == "default" ? "" : connection.model },
                      set: { connection.model = $0 }
                    )
                  ) {
                    Text("Default from Claude Code").tag("")
                    if !connection.model.isEmpty && connection.model != "default"
                      && !model.claudeModels.contains(where: { $0.id == connection.model })
                    {
                      Text("Unavailable: \(connection.model)").tag(connection.model).disabled(true)
                    }
                    ForEach(model.claudeModels.filter { $0.id != "default" }) { item in
                      Text(item.name).tag(item.id).help(item.description)
                    }
                  }.pickerStyle(.inline)
                } label: {
                  HStack(spacing: 8) {
                    Text(
                      connection.model.isEmpty || connection.model == "default"
                        ? "Default from Claude Code"
                        : model.claudeModels.first(where: { $0.id == connection.model })?.name
                          ?? "Unavailable: \(connection.model)")
                    Image(systemName: "chevron.down").font(.caption.weight(.semibold))
                  }
                }.modifier(SettingsMenuStyle())
              }
              if let error = model.claudeModelsError {
                Text(error).font(.caption).foregroundStyle(.secondary)
              }
            }
          } else {
            subscriptionSignIn
          }
        } else if selected != .disabled {
          SettingsTextField(
            title: "Model ID", text: $connection.model, prompt: "Enter the provider’s model ID"
          )
          .autocorrectionDisabled()
          if selected == .local || selected == .ollama || selected == .compatible {
            SettingsTextField(title: "Base URL", text: $connection.baseURL)
              .autocorrectionDisabled()
            Text(
              [.local, .ollama].contains(selected)
                ? "Use an OpenAI-compatible server, such as http://localhost:1234/v1 or http://localhost:11434/v1."
                : "Enter the HTTPS base URL for an OpenAI-compatible API, including /v1 if required."
            )
            .font(.caption).foregroundStyle(.secondary)
          }
          SettingsTextField(
            title: selected.requiresKey ? "API key" : "API key (optional)", text: $key,
            prompt: "Leave blank to keep the saved key", secure: true)
          HStack {
            Text("Keys are stored in your Mac’s Keychain.").font(.caption).foregroundStyle(
              .secondary)
            Spacer()
            Button("Remove saved key", role: .destructive) {
              do {
                try model.credentials.remove(connection)
                key = ""
                if connection.provider == model.provider
                  && connection.baseURL == model.connection.baseURL
                {
                  try model.connectAPI()
                }
              } catch { self.error = error.localizedDescription }
            }.controlSize(.small)
          }
        }
        if [.chatGPT, .claude].contains(selected), model.provider == selected, model.signedIn {
          Button("Sign out") {
            saving = true
            error = nil
            Task {
              defer { saving = false }
              await model.signOut()
              error = model.error
            }
          }
          .disabled(
            model.busy || model.creatingSpace || model.connecting || model.loginPending
              || model.subscriptionModelsLoading || model.claudeModelsLoading
          )
          .help(
            selected == .claude
              ? "Also signs out of Claude Code on this Mac." : "Sign out of ChatGPT in Tonk."
          )
          .frame(maxWidth: .infinity, alignment: .trailing)
        }
      }.controlSize(.regular)
      Divider()
      Text(
        "Changing provider or model starts a new conversation. Your previous transcript is saved on this Mac, and the attached space stays open."
      )
      .font(.caption).foregroundStyle(.secondary)
      if let error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled) }
      HStack {
        Button("Advanced…") { showAdvancedSettings = true }
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
        Button(saving ? "Saving…" : "Apply") {
          saving = true
          error = nil
          connection.model = connection.model.trimmingCharacters(in: .whitespacesAndNewlines)
          connection.baseURL = connection.baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
          Task {
            defer { saving = false }
            do {
              try await model.configureProvider(
                selected, connection: connection, key: key, codexExecutable: codexExecutable)
              if selected == .chatGPT, !model.connected, let connectionError = model.error {
                error = connectionError
                return
              }
              dismiss()
            } catch { self.error = error.localizedDescription }
          }
        }.nativeControl(prominent: true).keyboardShortcut(.defaultAction)
          .disabled(model.loginPending || model.connecting)
      }.controlSize(.regular)
    }
    .padding(24).frame(width: 540).nativeControl().disabled(saving)
    .task { if !model.connected { await model.connect() } }
    .interactiveDismissDisabled(saving)
    .sheet(isPresented: $showAdvancedSettings) { AdvancedSettingsView() }
    .onChange(of: selected) { _, value in
      if connection.provider != value {
        connection = model.saved.connections?[value.rawValue] ?? ModelConnection(provider: value)
      }
      key = ""
      error = nil
      startedSignIn = false
    }
    .onChange(of: model.error) { _, value in
      if startedSignIn { error = value }
    }
    .onChange(of: connection.baseURL) { _, _ in key = "" }
  }

  @ViewBuilder
  private var subscriptionSignIn: some View {
    if model.loginPending && model.provider == selected {
      HStack {
        HStack(spacing: 8) {
          ProgressView().controlSize(.small)
          Text("Continue in your browser")
            .font(.caption).foregroundStyle(.secondary)
        }
        Spacer()
        Button("Cancel sign-in") { Task { await model.cancelLogin() } }
      }
    } else {
      Button(selected == .claude ? "Sign in with Claude" : "Sign in with ChatGPT") {
        saving = true
        startedSignIn = true
        error = nil
        Task {
          defer { saving = false }
          do {
            try await model.configureProvider(
              selected, connection: connection, key: "", codexExecutable: codexExecutable)
            guard model.connected else {
              error = model.error ?? "Could not connect. Try signing in again."
              return
            }
            if !model.signedIn { await model.signIn() }
            error = model.error
          } catch { self.error = error.localizedDescription }
        }
      }
      .nativeControl(prominent: true)
      .disabled(model.busy || model.creatingSpace || model.connecting || model.loginPending)
      .frame(maxWidth: .infinity, alignment: .trailing)
    }
  }

  private func chooseCodexExecutable() {
    let panel = NSOpenPanel()
    panel.title = "Choose Codex CLI"
    panel.prompt = "Choose"
    panel.canChooseDirectories = false
    panel.allowsMultipleSelection = false
    panel.showsHiddenFiles = true
    if let codexExecutable {
      panel.directoryURL = URL(fileURLWithPath: codexExecutable).deletingLastPathComponent()
    }
    panel.begin { response in
      guard response == .OK, let url = panel.url else { return }
      do {
        _ = try CodexInstallation.validate(url.path)
        codexExecutable = url.path
        error = nil
      } catch { self.error = error.localizedDescription }
    }
  }
}

private struct SettingsMenuStyle: ViewModifier {
  func body(content: Content) -> some View {
    content.menuStyle(.borderlessButton).menuIndicator(.hidden)
      .padding(.horizontal, 8).padding(.vertical, 4)
      .controlSurface(radius: 12).fixedSize()
  }
}

private struct SettingsTextField: View {
  let title: String
  @Binding var text: String
  var prompt = ""
  var secure = false
  @FocusState private var focused: Bool

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text(title)
      Group {
        if secure {
          SecureField(title, text: $text, prompt: Text(prompt))
        } else {
          TextField(title, text: $text, prompt: Text(prompt))
        }
      }.textFieldStyle(.plain).focused($focused).accessibilityLabel(title)
        .padding(10).controlSurface(radius: 12)
        .overlay {
          RoundedRectangle(cornerRadius: 12)
            .strokeBorder(focused ? Color.accentColor : .clear, lineWidth: 2)
            .allowsHitTesting(false)
        }
    }
  }
}
