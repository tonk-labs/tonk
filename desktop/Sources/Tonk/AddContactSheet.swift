import HarnessCore
import SwiftUI

/// Adding a contact starts from one search field. A complete email is looked up
/// in the account directory as it is typed and a did:key is taken as entered;
/// choosing the result moves on to the contact's name and labels.
struct AddContactSheet: View {
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  /// The saved contact's key, so the list can select it.
  var onAdded: (String) -> Void = { _ in }
  @StateObject private var lookup = ContactLookupModel()
  @State private var lookupTask: Task<Void, Never>?
  @State private var chosen: Choice?
  @State private var name = ""
  @State private var labels: [String] = []
  @State private var error: String?
  @State private var account: String?
  @FocusState private var nameFocused: Bool
  @Environment(\.dismiss) private var dismiss

  struct Choice: Equatable {
    let did: String
    let identity: EmailContactIdentity?
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Add Contact").font(.headline)
      if let chosen {
        details(chosen)
      } else {
        search
      }
    }
    .padding(22).frame(width: 440)
    .onAppear { account = access.selfDid }
    .onChange(of: lookup.input) { _, _ in scheduleLookup() }
    .onChange(of: access.selfDid) { _, _ in dismiss() }
    .onDisappear {
      lookupTask?.cancel()
      lookup.invalidate()
    }
  }

  // MARK: Search

  private var search: some View {
    VStack(alignment: .leading, spacing: 16) {
      VStack(alignment: .leading, spacing: 6) {
        GlassSearchField(title: "Email or did:key", text: $lookup.input, focusRequest: 1)
          .onSubmit(submitSearch)
        ContactResultsCard {
          results
        }
      }
      HStack {
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
      }
    }
  }

  @ViewBuilder
  private var results: some View {
    let trimmed = lookup.input.trimmingCharacters(in: .whitespacesAndNewlines)
    if trimmed.isEmpty {
      ContactResultMessage(
        "Search by the email on someone’s Tonk account, or paste their did:key.")
    } else if let did = lookup.manualDid {
      candidate(Choice(did: did, identity: nil))
    } else if lookup.isLoading {
      HStack(spacing: 8) {
        ProgressView().controlSize(.small)
        Text("Looking up \(trimmed)…").foregroundStyle(.secondary).lineLimit(1)
      }.padding(.horizontal, 10).frame(height: 44)
    } else if let identity = lookup.identity {
      candidate(Choice(did: identity.rootDid, identity: identity))
    } else if let result = lookup.result {
      switch result {
      case .notFound:
        EmailInviteResult(email: trimmed, spaces: spaces, access: access, account: account)
      case .pending:
        ContactResultMessage("\(trimmed) is registered but not confirmed yet.")
      case .suspended:
        ContactResultMessage("The account for \(trimmed) is suspended.")
      case .found: EmptyView()
      }
    } else if let error = lookup.error {
      ContactResultMessage(error, isError: true)
    } else if trimmed.contains("@") {
      ContactResultMessage("Keep typing the full email address.")
    } else {
      ContactResultMessage("Enter an email address or a did:key.")
    }
  }

  @ViewBuilder
  private func candidate(_ choice: Choice) -> some View {
    if let existing = access.contacts.first(where: { $0.id == choice.did }) {
      ContactResultRow(
        title: existing.name, detail: choice.identity?.email ?? choice.did,
        trailing: "Already a contact")
    } else {
      Button {
        choose(choice)
      } label: {
        ContactResultRow(
          title: choice.identity?.email ?? "Account key",
          detail: choice.did, trailing: nil, chevron: true)
      }.buttonStyle(ContactResultButtonStyle())
        .help("Add this account as a contact")
    }
  }

  private func scheduleLookup() {
    lookupTask?.cancel()
    guard lookup.isCompleteEmail else { return }
    lookupTask = Task {
      // Wait for a pause in typing so partial addresses are not sent.
      try? await Task.sleep(for: .milliseconds(500))
      guard !Task.isCancelled else { return }
      await lookup.lookup()
    }
  }

  private func submitSearch() {
    if let did = lookup.did {
      guard !access.contacts.contains(where: { $0.id == did }) else { return }
      choose(Choice(did: did, identity: lookup.identity))
    } else if !lookup.isLoading, lookup.result == nil {
      lookupTask?.cancel()
      lookupTask = Task { await lookup.lookup() }
    }
  }

  private func choose(_ choice: Choice) {
    chosen = choice
    error = nil
    nameFocused = true
  }

  // MARK: Details

  private func details(_ choice: Choice) -> some View {
    VStack(alignment: .leading, spacing: 16) {
      HStack(spacing: 10) {
        Image(systemName: choice.identity == nil ? "key" : "envelope")
          .frame(width: 28, height: 28).background(.quaternary, in: Circle())
          .accessibilityHidden(true)
        VStack(alignment: .leading, spacing: 2) {
          Text(choice.identity?.email ?? "Account key").lineLimit(1)
          Text(choice.did).font(.system(size: 11, design: .monospaced))
            .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            .textSelection(.enabled)
        }
        Spacer(minLength: 8)
        Button("Change") { chosen = nil }
      }
      .padding(10).background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
      VStack(alignment: .leading, spacing: 6) {
        Text("Name").font(.caption).foregroundStyle(.secondary)
        TextField("Name", text: $name).textFieldStyle(.roundedBorder)
          .focused($nameFocused).accessibilityLabel("Contact name")
          .onSubmit { save(choice) }
      }
      VStack(alignment: .leading, spacing: 6) {
        Text("Labels").font(.caption).foregroundStyle(.secondary)
        ContactLabelsField(labels: $labels)
      }
      Text("Contacts and labels are saved on this Mac. Adding a contact does not give them access.")
        .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
      if let error { Text(error).foregroundStyle(.red) }
      HStack {
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
        Button("Add Contact") { save(choice) }.keyboardShortcut(.defaultAction)
          .disabled(!canSave)
      }
    }
  }

  private var canSave: Bool {
    account != nil && account == access.selfDid
      && !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  private func save(_ choice: Choice) {
    guard canSave else { return }
    let contact = DesktopContact(
      id: choice.did, name: name.trimmingCharacters(in: .whitespacesAndNewlines),
      labels: labels, emailIdentity: choice.identity)
    do {
      try access.saveContact(contact)
      onAdded(contact.id)
      dismiss()
    } catch { self.error = error.localizedDescription }
  }
}

/// The search result shown under the field, like a menu's single row.
private struct ContactResultsCard<Content: View>: View {
  @ViewBuilder var content: Content
  var body: some View {
    content.frame(maxWidth: .infinity, alignment: .leading).padding(4)
      .background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 12))
  }
}

private struct ContactResultMessage: View {
  let text: String
  var isError = false
  init(_ text: String, isError: Bool = false) {
    self.text = text
    self.isError = isError
  }
  var body: some View {
    Text(text).foregroundStyle(isError ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
      .fixedSize(horizontal: false, vertical: true)
      .padding(.horizontal, 10).padding(.vertical, 12)
  }
}

private struct ContactResultRow: View {
  let title: String
  let detail: String
  let trailing: String?
  var chevron = false
  var body: some View {
    HStack(spacing: 10) {
      Text(AccessText.initials(title.split(separator: "@").first.map(String.init) ?? title))
        .font(.system(size: 10, weight: .semibold))
        .frame(width: 28, height: 28).background(.quaternary, in: Circle())
      VStack(alignment: .leading, spacing: 2) {
        Text(title).lineLimit(1)
        Text(detail).font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
          .lineLimit(1).truncationMode(.middle)
      }
      Spacer(minLength: 8)
      if let trailing { Text(trailing).font(.caption).foregroundStyle(.secondary) }
      if chevron { Image(systemName: "chevron.right").foregroundStyle(.tertiary) }
    }.padding(.horizontal, 10).frame(height: 48).contentShape(Rectangle())
  }
}

private struct ContactResultButtonStyle: ButtonStyle {
  @State private var hovering = false
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .background(
        Color.accentColor.opacity(configuration.isPressed ? 0.22 : hovering ? 0.12 : 0),
        in: RoundedRectangle(cornerRadius: 8)
      )
      .onHover { hovering = $0 }
  }
}

/// No account has this email: offer to invite it into a space, which signs them up.
private struct EmailInviteResult: View {
  let email: String
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  let account: String?
  @State private var expanded = false
  @State private var spaceID = ""
  @State private var sending = false
  @State private var sent = false
  @State private var error: String?

  var body: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 10) {
        Image(systemName: "person.crop.circle.badge.questionmark").font(.system(size: 18))
          .foregroundStyle(.secondary).frame(width: 28, height: 28).accessibilityHidden(true)
        VStack(alignment: .leading, spacing: 2) {
          Text(email).lineLimit(1)
          Text(sent ? "Invitation sent" : "No Tonk account yet").font(.caption)
            .foregroundStyle(.secondary)
        }
        Spacer(minLength: 8)
        if !expanded && !sent {
          Button("Invite by Email…") { expanded = true }.disabled(spaces.isEmpty)
        }
      }.padding(.horizontal, 10).frame(minHeight: 48)
      if sent {
        Text("They can create an account, confirm their email, and join the space.")
          .font(.caption).foregroundStyle(.secondary).padding(.horizontal, 10).padding(.bottom, 8)
      } else if spaces.isEmpty {
        Text("Create or join a synced space before inviting someone.")
          .font(.caption).foregroundStyle(.secondary).padding(.horizontal, 10).padding(.bottom, 8)
      } else if expanded {
        VStack(alignment: .leading, spacing: 8) {
          HStack {
            Picker("Space", selection: $spaceID) {
              Text("Choose a space").tag("")
              ForEach(spaces) { space in Text(space.title).tag(space.id) }
            }.labelsHidden().disabled(sending)
            Button {
              send()
            } label: {
              HStack(spacing: 6) {
                if sending { ProgressView().controlSize(.small) }
                Text(sending ? "Sending" : "Send Invite")
              }
            }.disabled(sending || spaceID.isEmpty || account == nil || account != access.selfDid)
          }
          Text("They’ll get an email and join the space after confirming this address.")
            .font(.caption).foregroundStyle(.secondary)
          if let error { Text(error).font(.caption).foregroundStyle(.red) }
        }.padding(.horizontal, 10).padding(.bottom, 10)
      }
    }
  }

  private func send() {
    guard let space = spaces.first(where: { $0.id == spaceID }) else { return }
    sending = true
    error = nil
    Task {
      defer { sending = false }
      do {
        try await access.sendEmailInvitation(email: email, space: space)
        sent = true
      } catch { self.error = error.localizedDescription }
    }
  }
}

/// Labels as removable chips, with a field that adds one on Return and
/// suggestions that add with a click.
struct ContactLabelsField: View {
  @Binding var labels: [String]
  @State private var draft = ""
  static let suggestions = ["Team", "Work", "Family", "Verified"]

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      HStack(spacing: 6) {
        ForEach(Array(labels.enumerated()), id: \.offset) { index, label in
          Button {
            labels.remove(at: index)
          } label: {
            HStack(spacing: 4) {
              Text(label)
              Image(systemName: "xmark").font(.system(size: 9))
            }.padding(.horizontal, 9).frame(height: 24)
          }.buttonStyle(.plain).background(.quaternary, in: Capsule())
            .accessibilityLabel("Remove label \(label)")
        }
        TextField(labels.isEmpty ? "Add a label" : "Add another", text: $draft)
          .textFieldStyle(.plain).frame(minWidth: 80)
          .onSubmit { add(draft) }
          .onChange(of: draft) { _, value in
            if value.hasSuffix(",") { add(String(value.dropLast())) }
          }
      }
      .padding(.horizontal, 6).frame(minHeight: 32)
      .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.quaternary))
      let remaining = Self.suggestions.filter { suggestion in
        !labels.contains { $0.caseInsensitiveCompare(suggestion) == .orderedSame }
      }
      if !remaining.isEmpty {
        HStack(spacing: 6) {
          Text("Suggested:").foregroundStyle(.secondary)
          ForEach(remaining, id: \.self) { suggestion in
            Button(suggestion) { add(suggestion) }.buttonStyle(.plain)
              .padding(.horizontal, 8).frame(height: 22)
              .overlay(
                Capsule().strokeBorder(.tertiary, style: StrokeStyle(lineWidth: 1, dash: [3])))
              .help("Add the label \(suggestion)")
          }
        }.font(.system(size: 12))
      }
    }
  }

  private func add(_ label: String) {
    let trimmed = label.trimmingCharacters(in: .whitespacesAndNewlines)
    draft = ""
    guard !trimmed.isEmpty,
      !labels.contains(where: { $0.caseInsensitiveCompare(trimmed) == .orderedSame })
    else { return }
    labels.append(trimmed)
  }
}
