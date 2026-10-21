import HarnessCore
import SwiftUI

struct DesktopAccountView: View {
  @ObservedObject var access: AccessModel
  @ObservedObject var model: HarnessModel
  var onSignOut: () -> Void
  var showsChat = false
  @State private var name = ""
  @State private var saving = false
  @State private var error: String?
  @State private var confirming = false
  @State private var saveTask: Task<Void, Never>?

  var body: some View {
    ScrollViewReader { proxy in
      ScrollView {
        VStack(alignment: .leading, spacing: 32) {
          VStack(alignment: .leading, spacing: 8) {
            Text("Your details").fontWeight(.semibold)
            VStack(spacing: 0) {
              HStack {
                Text("Display name").frame(width: 120, alignment: .leading)
                TextField("Display name", text: $name).textFieldStyle(.roundedBorder)
                  .onSubmit { scheduleSave(delay: false) }
              }.padding(16)
              Divider()
              HStack {
                Text("Email").frame(width: 120, alignment: .leading)
                Text(
                  access.accountEmail
                    ?? (access.accountDetailsLoaded ? "No email registered" : "Not available")
                ).foregroundStyle(.secondary)
                Spacer(minLength: 0)
              }.padding(16)
            }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
            Text(
              saving
                ? "Saving…"
                : "Your name shows to people and agents in your spaces. Changes save as you type."
            )
            .font(.system(size: 12)).foregroundStyle(.secondary)
            if let error {
              Text(error).foregroundStyle(.red)
              Button("Try Again") { scheduleSave(delay: false) }
            }
          }
          VStack(alignment: .leading, spacing: 8) {
            Text("Passkeys").fontWeight(.semibold)
            VStack(alignment: .leading, spacing: 0) {
              ForEach(displayedPasskeys) { passkey in
                if passkey.id != displayedPasskeys.first?.id { Divider() }
                HStack(spacing: 12) {
                  DesktopGlyph(name: "r2Key", size: 16).foregroundStyle(.secondary)
                  VStack(alignment: .leading, spacing: 3) {
                    Text(passkey.createdOn)
                    if let date = passkey.createdAt {
                      Text("Created \(date.formatted(date: .abbreviated, time: .omitted))")
                        .font(.system(size: 12)).foregroundStyle(.secondary)
                    }
                  }
                  Spacer()
                }.padding(16)
              }
              if displayedPasskeys.isEmpty {
                Text(
                  access.accountDetailsLoaded ? "No passkeys listed" : "Passkey details unavailable"
                )
                .foregroundStyle(.secondary).padding(16)
              }
              Divider()
              Button(action: openAccount) {
                HStack(spacing: 8) {
                  DesktopGlyph(name: "r2Plus", size: 14)
                  Text("Add a Passkey in Browser…")
                }
              }.buttonStyle(.plain).padding(16)
            }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
            Text(
              "A passkey replaces a password. It can live on a device, a browser profile or a password manager."
            )
            .font(.system(size: 12)).foregroundStyle(.secondary)
          }
          if let detailsError = access.accountDetailsError {
            Text(detailsError).font(.system(size: 12)).foregroundStyle(.secondary)
            Button("Retry Account Details") { Task { await access.refreshAccount() } }
          }
          ProviderSettingsView(model: model, embedded: true).id("chat-settings")
            .background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 12))
          VStack(spacing: 0) {
            HStack {
              VStack(alignment: .leading, spacing: 3) {
                Text("Sign out on this Mac")
                Text("Disconnects this account. Your spaces stay on this Mac.")
                  .font(.system(size: 12)).foregroundStyle(.secondary)
              }
              Spacer()
              Button("Sign Out…") { confirming = true }
            }.padding(16)
            Divider()
            HStack {
              VStack(alignment: .leading, spacing: 3) {
                Text("Delete account")
                Text("Review permanent deletion in your browser account.")
                  .font(.system(size: 12)).foregroundStyle(.secondary)
              }
              Spacer()
              Button("Review Deletion…", action: openAccount).foregroundStyle(.red)
            }.padding(16)
          }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
        }.frame(maxWidth: 720).padding(28).frame(maxWidth: .infinity)
      }.font(.system(size: 13)).controlSize(.small).buttonBorderShape(.capsule)
        .onAppear {
          name = access.accountName ?? ""
          if showsChat { proxy.scrollTo("chat-settings", anchor: .top) }
        }
        .onChange(of: showsChat) { _, value in
          if value { proxy.scrollTo("chat-settings", anchor: .top) }
        }
        .onChange(of: name) { _, _ in scheduleSave() }
        .onChange(of: access.selfDid) { _, _ in
          saveTask?.cancel()
          name = access.accountName ?? ""
        }
        .onDisappear { scheduleSave(delay: false) }
        .confirmationDialog("Sign out on this Mac?", isPresented: $confirming) {
          Button("Sign Out", action: onSignOut)
        }
    }
  }

  private var displayedPasskeys: [AccountPasskey] {
    if access.accountDetailsLoaded { return access.accountPasskeys }
    guard let label = access.passkeyCreatedOn else { return [] }
    return [AccountPasskey(id: "local", createdOn: label, createdAt: access.passkeyCreatedAt)]
  }

  private func openAccount() {
    NSWorkspace.shared.open(RuntimeLocation.home.appendingPathComponent("account"))
  }

  /// Debounce keystrokes, serialize writes, and never carry a draft across accounts.
  private func scheduleSave(delay: Bool = true) {
    guard !saving else { return }
    saveTask?.cancel()
    guard let root = access.selfDid else { return }
    saveTask = Task {
      if delay {
        do { try await Task.sleep(for: .milliseconds(500)) } catch { return }
      }
      guard root == access.selfDid, !Task.isCancelled else { return }
      let submitted = name.trimmingCharacters(in: .whitespacesAndNewlines)
      guard !submitted.isEmpty, submitted != access.accountName else { return }
      saving = true
      do {
        try await access.updateDisplayName(submitted)
        if root == access.selfDid { error = nil }
      } catch {
        if root == access.selfDid { self.error = error.localizedDescription }
      }
      saving = false
      if root == access.selfDid && submitted != name.trimmingCharacters(in: .whitespacesAndNewlines)
      {
        scheduleSave()
      }
    }
  }
}

/// In-window destination shared by both account footer controls.
struct DesktopSettingsView: View {
  @ObservedObject var model: HarnessModel
  @ObservedObject var access: AccessModel
  var onSignOut: () -> Void
  var showsChat = false
  var body: some View {
    DesktopAccountView(access: access, model: model, onSignOut: onSignOut, showsChat: showsChat)
      .frame(maxWidth: .infinity, maxHeight: .infinity).nativeControl()
  }
}
