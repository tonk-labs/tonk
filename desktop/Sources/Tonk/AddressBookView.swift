import HarnessCore
import SwiftUI

struct DesktopContact: Codable, Identifiable, Equatable {
  var id: String
  var name: String
  var labels: [String]
  var emailIdentity: EmailContactIdentity? = nil
}

/// Everyone who can reach your spaces, people and agents alike, in the manner
/// of Contacts: pick someone, see what they can reach, act on it in place.
struct AddressBookView: View {
  @ObservedObject var access: AccessModel
  let spaces: [TonkSpace]
  var searchFocusRequest = 0
  /// Opens a space's own page, where its links and settings are managed.
  var onOpenSpace: (TonkSpace) -> Void = { _ in }
  @State private var search = ""
  @State private var selected: String?
  @State private var editing: DesktopContact?
  @State private var inviting: DesktopContact?
  @State private var adding = false

  private var entries: [AddressBookEntry] {
    AddressBookEntry.entries(
      contacts: access.contacts, grants: access.connections, hosted: access.hostedAgents
    )
    .filter { $0.matches(search) }
  }

  var body: some View {
    // A fixed list beside a flexible detail: a split view's minimums would
    // push the window's own sidebar off screen at the 800-point default size.
    HStack(spacing: 0) {
      VStack(spacing: 0) {
        HStack(spacing: 8) {
          GlassSearchField(
            title: "Name, email or key", text: $search, focusRequest: searchFocusRequest)
          Button {
            adding = true
          } label: {
            Label("Add contact", systemImage: "plus").labelStyle(.iconOnly)
              .frame(width: 20, height: 20)
          }.help("Add contact")
        }.padding(.horizontal, 12).padding(.top, 12).padding(.bottom, 4)
        List(entries, selection: $selected) { entry in
          AddressBookRow(entry: entry).tag(entry.id).listRowSeparator(.hidden)
            .contextMenu {
              if let contact = entry.contact {
                Button("Edit Contact…") { editing = contact }
                if contact.id != access.selfDid {
                  Button("Invite to a Space…") { inviting = contact }.disabled(spaces.isEmpty)
                }
              }
            }
        }
        // The sidebar style insets the selection as a rounded rectangle, as in frame 11.
        .listStyle(.sidebar).scrollContentBackground(.hidden)
        if let error = access.errors["account"] {
          VStack(alignment: .leading, spacing: 6) {
            Text("Couldn’t load connected agents. " + error)
              .font(.system(size: 12)).foregroundStyle(.secondary)
            Button("Try Again") { Task { await access.refreshAccount() } }.controlSize(.small)
          }.padding(12).frame(maxWidth: .infinity, alignment: .leading)
        }
      }.frame(width: 228)
      Divider()
      if let entry = entries.first(where: { $0.id == selected }) {
        AddressBookDetail(
          entry: entry, access: access, spaces: spaces,
          onEdit: { editing = $0 }, onInvite: { inviting = $0 }, onOpenSpace: onOpenSpace
        ).id(entry.id)
      } else {
        Text(
          !search.isEmpty && entries.isEmpty
            ? "No matching contacts" : entries.isEmpty ? "No contacts yet" : "Select a contact"
        )
        .foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    }.font(.system(size: 13))
      .sheet(item: $editing) { contact in ContactEditor(contact: contact, access: access) }
      .sheet(item: $inviting) { contact in
        ContactInviteSheet(contact: contact, spaces: spaces, access: access)
      }
      .sheet(isPresented: $adding) {
        AddContactSheet(spaces: spaces, access: access) { did in
          search = ""
          selected = did
        }
      }
      .onChange(of: entries.map(\.id), initial: true) { _, ids in
        if selected == nil || !ids.contains(selected!) { selected = ids.first }
      }
  }
}

/// Copies text and says so for a moment, so the click visibly did something.
struct CopyButton<Label: View>: View {
  let text: String
  @ViewBuilder var label: (_ copied: Bool) -> Label
  @State private var copied = false

  var body: some View {
    Button {
      NSPasteboard.general.clearContents()
      NSPasteboard.general.setString(text, forType: .string)
      copied = true
      AccessibilityNotification.Announcement("Copied").post()
    } label: {
      label(copied)
    }
    .task(id: copied) {
      guard copied else { return }
      try? await Task.sleep(for: .seconds(1.5))
      copied = false
    }
  }
}

private struct AddressBookAvatar: View {
  let entry: AddressBookEntry
  var size: CGFloat = 24

  var body: some View {
    switch entry.kind {
    case .person:
      Text(AccessText.initials(entry.name)).font(.system(size: size * 0.4, weight: .semibold))
        .frame(width: size, height: size).background(.quaternary, in: Circle())
    case .agent:
      DesktopGlyph(name: "r2IconBot", size: size * 0.55)
        .frame(width: size, height: size)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: size * 0.25))
    }
  }
}

private struct AddressBookRow: View {
  let entry: AddressBookEntry

  var body: some View {
    HStack(spacing: 10) {
      AddressBookAvatar(entry: entry)
      VStack(alignment: .leading, spacing: 3) {
        Text(entry.name).lineLimit(1)
        Group {
          if let email = entry.email?.email {
            Text(email).lineLimit(1).truncationMode(.middle)
          } else if let did = entry.did {
            HStack(spacing: 4) {
              Text(did).font(.system(size: 11, design: .monospaced))
                .lineLimit(1).truncationMode(.middle)
              CopyButton(text: did) { copied in
                Image(systemName: copied ? "checkmark" : "doc.on.doc")
                  .font(.system(size: 10)).frame(width: 12, height: 12)
              }
              .buttonStyle(.plain).help("Copy key")
              .accessibilityLabel("Copy key for \(entry.name)")
            }
          } else {
            Text("No key reported")
          }
        }.font(.system(size: 11)).foregroundStyle(.secondary)
      }
    }.padding(.vertical, 4)
  }
}

/// Contacts' launching-off points: the actions you take with someone, as a row
/// of round buttons under their name.
private struct ContactActionLabel: View {
  let title: String
  let systemImage: String

  var body: some View {
    VStack(spacing: 4) {
      Image(systemName: systemImage).font(.system(size: 14))
        .frame(width: 34, height: 34).background(.quaternary, in: Circle())
      Text(title).font(.system(size: 11))
    }.frame(minWidth: 56).contentShape(Rectangle())
  }
}

private struct ContactAction: View {
  let title: String
  let systemImage: String
  let action: () -> Void

  var body: some View {
    Button(action: action) { ContactActionLabel(title: title, systemImage: systemImage) }
      .buttonStyle(.plain).accessibilityLabel(title)
  }
}

private struct AddressBookDetail: View {
  let entry: AddressBookEntry
  @ObservedObject var access: AccessModel
  let spaces: [TonkSpace]
  let onEdit: (DesktopContact) -> Void
  let onInvite: (DesktopContact) -> Void
  let onOpenSpace: (TonkSpace) -> Void

  private var isSelf: Bool { entry.did != nil && entry.did == access.selfDid }

  /// Spaces this key is a member of, as far as the loaded rosters show.
  private var memberSpaces: [(TonkSpace, SpaceAccess)] {
    guard let did = entry.did else { return [] }
    return spaces.compactMap { space in
      guard let state = access.spaces[space.id], state.member(did) != nil else { return nil }
      return (space, state)
    }
  }

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 22) {
        header
        actions
        spacesSection
        identitySection
        if let contact = entry.contact, access.contacts.contains(where: { $0.id == contact.id }) {
          ContactLabelsSection(contact: contact, access: access)
        }
      }
      .frame(maxWidth: 680, alignment: .leading).padding(.horizontal, 20).padding(.vertical, 18)
      .frame(maxWidth: .infinity, alignment: .leading)
    }.frame(maxWidth: .infinity)
  }

  private var header: some View {
    HStack(spacing: 14) {
      AddressBookAvatar(entry: entry, size: 52)
      VStack(alignment: .leading, spacing: 2) {
        Text(entry.name).font(.system(size: 17, weight: .semibold)).textSelection(.enabled)
        Text(subtitle).font(.system(size: 12)).foregroundStyle(.secondary)
      }
    }
  }

  private var subtitle: String {
    switch entry.kind {
    case .agent:
      if entry.grants.contains(where: \.isTerminalAuthorization) { return "Agent · Tonk CLI" }
      return entry.grants.isEmpty && entry.hosted == nil
        ? "Agent" : "Agent · connected with access you gave"
    case .person:
      return isSelf ? "You" : "Person"
    }
  }

  @ViewBuilder private var actions: some View {
    HStack(spacing: 6) {
      if let contact = entry.contact, !isSelf {
        ContactAction(title: "Invite", systemImage: "person.crop.circle.badge.plus") {
          onInvite(contact)
        }.disabled(spaces.isEmpty).help("Invite to a space with a link only they can use")
      }
      if let did = entry.did {
        CopyButton(text: did) { copied in
          ContactActionLabel(
            title: copied ? "Copied" : "Copy Key", systemImage: copied ? "checkmark" : "key")
        }.buttonStyle(.plain).accessibilityLabel("Copy Key").help("Copy their did:key")
      }
      if let contact = entry.contact {
        ContactAction(title: "Edit", systemImage: "pencil") { onEdit(contact) }
      }
    }
  }

  private var spacesSection: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("Spaces").font(.caption).foregroundStyle(.secondary)
      let members = memberSpaces
      let hostedSpaces = entry.hosted?.spaces ?? []
      if members.isEmpty && entry.grants.isEmpty && hostedSpaces.isEmpty {
        Text(
          isSelf
            ? "You’re not in any listed space."
            : "Not in any of your spaces yet. Invite \(entry.name) to share one."
        )
        .foregroundStyle(.secondary)
      } else {
        VStack(spacing: 0) {
          ForEach(members, id: \.0.id) { space, state in
            if space.id != members.first?.0.id { Divider() }
            MemberSpaceRow(
              name: entry.name, did: entry.did ?? "", isSelf: isSelf, space: space, state: state,
              access: access, onOpenSpace: onOpenSpace)
          }
          ForEach(entry.grants, id: \.id) { grant in
            if !members.isEmpty || grant.id != entry.grants.first?.id { Divider() }
            GrantRow(
              grant: grant, space: spaces.first { $0.subject == grant.subject }, access: access)
          }
          if let agent = entry.hosted {
            ForEach(hostedSpaces, id: \.subject) { space in
              if !members.isEmpty || !entry.grants.isEmpty
                || space.subject != hostedSpaces.first?.subject
              {
                Divider()
              }
              HostedSpaceRow(
                agent: agent, name: entry.name, space: space,
                listed: spaces.first { $0.subject == space.subject }, access: access)
            }
          }
        }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
      }
    }
  }

  /// Email and key, each copyable, as the card's contact details.
  @ViewBuilder private var identitySection: some View {
    let identity = entry.email
    if identity != nil || entry.did != nil {
      VStack(alignment: .leading, spacing: 8) {
        VStack(spacing: 0) {
          if let identity {
            IdentityRow(label: "email", value: identity.email, monospaced: false)
          }
          if let did = entry.did {
            if identity != nil { Divider() }
            IdentityRow(label: "did:key", value: did, monospaced: true)
          }
        }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
        if let identity {
          Text(
            "Email checked with \(identity.directory.host ?? identity.directory.absoluteString) on \(identity.checkedAt.formatted(date: .abbreviated, time: .omitted))."
          )
          .font(.caption).foregroundStyle(.secondary)
        }
      }
    }
  }
}

private struct IdentityRow: View {
  let label: String
  let value: String
  let monospaced: Bool

  var body: some View {
    HStack(alignment: .firstTextBaseline, spacing: 12) {
      VStack(alignment: .leading, spacing: 3) {
        Text(label).font(.caption).foregroundStyle(.secondary)
        Text(value).font(monospaced ? .system(size: 12, design: .monospaced) : .system(size: 13))
          .textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
      }
      Spacer(minLength: 4)
      CopyButton(text: value) { copied in
        Text(copied ? "Copied" : "Copy").frame(minWidth: 44)
      }.accessibilityLabel("Copy \(label)")
    }.padding(.horizontal, 14).padding(.vertical, 10)
  }
}

/// One space someone is in: open it to manage its links, or remove them.
private struct MemberSpaceRow: View {
  let name: String
  let did: String
  let isSelf: Bool
  let space: TonkSpace
  let state: SpaceAccess
  @ObservedObject var access: AccessModel
  let onOpenSpace: (TonkSpace) -> Void
  @State private var confirming = false

  var body: some View {
    let founder = state.member(did)?.isFounder == true
    HStack(spacing: 8) {
      VStack(alignment: .leading, spacing: 3) {
        Text(space.title)
        Text((founder ? "Founder" : "Member") + (isSelf ? " · You" : ""))
          .font(.system(size: 12)).foregroundStyle(.secondary)
      }
      Spacer(minLength: 8)
      // The founder holds the space itself; leaving is not a removal.
      if !isSelf && !founder {
        Button("Remove…") { confirming = true }
          .disabled(access.working.contains("member:" + did))
          .confirmationDialog("Remove \(name) from \(space.title)?", isPresented: $confirming) {
            Button("Remove", role: .destructive) {
              Task { await access.removeMember(did, name: name, from: space) }
            }
          } message: {
            Text(
              "\(name) loses access to \(space.title), whichever link they joined with. Other people keep theirs."
            )
          }
      }
      Button("Manage Space") { onOpenSpace(space) }
        .help("Open \(space.title) to manage its people, links and settings")
    }.padding(.horizontal, 14).padding(.vertical, 10)
  }
}

/// Access an agent connected through, with the way to revoke it.
private struct GrantRow: View {
  let grant: AgentConnection
  let space: TonkSpace?
  @ObservedObject var access: AccessModel
  @State private var confirming = false

  private var link: AccessLink {
    AccessLink(kind: .agent, subject: grant.subject, target: grant.id, usedBy: nil)
  }

  var body: some View {
    HStack(spacing: 12) {
      VStack(alignment: .leading, spacing: 3) {
        Text(space?.title ?? grant.spaceName ?? "A space you no longer list")
        Text(
          [
            grant.isTerminalAuthorization ? "Tonk CLI" : "Access grant", "Read and write",
            AccessLink.expiry(
              Date(timeIntervalSince1970: grant.expiresAt), status: grant.status, now: Date()),
          ].joined(separator: " · ")
        ).font(.system(size: 12)).foregroundStyle(.secondary)
        if !grant.installations.isEmpty {
          Text("Clients: " + AccessText.list(grant.installations.map(\.name)))
            .font(.system(size: 12)).foregroundStyle(.secondary)
        }
      }
      Spacer(minLength: 8)
      Button("Revoke…") { confirming = true }
        .disabled(space == nil || access.working.contains(link.id))
    }.padding(.horizontal, 14).padding(.vertical, 10)
      .confirmationDialog("Revoke this access?", isPresented: $confirming) {
        Button("Revoke Access", role: .destructive) {
          guard let space else { return }
          Task { await access.revoke(link, space: space) }
        }
      } message: {
        Text(
          "Every client using it loses access to \(space?.title ?? "the space"). Other access stays as it is."
        )
      }
  }
}

/// A space a ChatGPT or Claude agent can reach, with the way to revoke it.
private struct HostedSpaceRow: View {
  let agent: HostedAgent
  let name: String
  let space: HostedAgentSpace
  let listed: TonkSpace?
  @ObservedObject var access: AccessModel
  @State private var confirming = false

  private var title: String { listed?.title ?? space.name ?? "A space you no longer list" }

  var body: some View {
    HStack(spacing: 12) {
      VStack(alignment: .leading, spacing: 3) {
        Text(title)
        Text(
          [
            space.canWrite ? "Read and write" : "Read",
            AccessLink.expiry(
              space.expiresAt.map(Date.init(timeIntervalSince1970:)), status: "active",
              now: Date()),
          ].joined(separator: " · ")
        ).font(.system(size: 12)).foregroundStyle(.secondary)
      }
      Spacer(minLength: 8)
      Button("Revoke…") { confirming = true }
        .disabled(access.working.contains(AccessModel.workingKey(space, of: agent)))
    }.padding(.horizontal, 14).padding(.vertical, 10)
      .confirmationDialog("Revoke this access?", isPresented: $confirming) {
        Button("Revoke Access", role: .destructive) {
          Task { await access.revoke(space, from: agent) }
        }
      } message: {
        Text("\(name) loses access to \(title). Other access stays as it is.")
      }
  }
}

/// Labels are added and removed in place; the full editor is for a contact's name.
private struct ContactLabelsSection: View {
  let contact: DesktopContact
  @ObservedObject var access: AccessModel
  @State private var saveError: String?
  @State private var addingLabel = false
  @State private var newLabel = ""
  @FocusState private var labelFocused: Bool

  var body: some View {
    VStack(alignment: .leading, spacing: 8) {
      Text("Labels").font(.caption).foregroundStyle(.secondary)
      ContactLabelLayout {
        ForEach(Array(contact.labels.enumerated()), id: \.offset) { index, label in
          Button {
            var updated = contact
            updated.labels.remove(at: index)
            save(updated)
          } label: {
            HStack(spacing: 4) {
              Text(label)
              Image(systemName: "xmark").font(.system(size: 9))
            }.padding(.horizontal, 9).frame(height: 24)
          }.buttonStyle(.plain).background(.quaternary, in: Capsule())
            .accessibilityLabel("Remove label \(label)")
        }
        if addingLabel {
          TextField("Label", text: $newLabel).textFieldStyle(.plain)
            .focused($labelFocused).frame(width: 96)
            .padding(.horizontal, 9).frame(height: 24)
            .overlay(Capsule().strokeBorder(Color.accentColor, lineWidth: 1))
            .onSubmit { addLabel(newLabel) }
            .onExitCommand { addingLabel = false }
            .onChange(of: labelFocused) { _, focused in
              if !focused { addLabel(newLabel) }
            }
        } else {
          Button("+ Add label") {
            newLabel = ""
            addingLabel = true
            labelFocused = true
          }.buttonStyle(.plain)
            .padding(.horizontal, 9).frame(height: 24)
            .overlay(
              Capsule().strokeBorder(.tertiary, style: StrokeStyle(lineWidth: 1, dash: [3]))
            )
        }
      }.font(.system(size: 12))
      if let saveError { Text(saveError).foregroundStyle(.red) }
    }
  }

  private func addLabel(_ label: String) {
    let trimmed = label.trimmingCharacters(in: .whitespacesAndNewlines)
    addingLabel = false
    newLabel = ""
    guard !trimmed.isEmpty,
      !contact.labels.contains(where: { $0.caseInsensitiveCompare(trimmed) == .orderedSame })
    else { return }
    var updated = contact
    updated.labels.append(trimmed)
    save(updated)
  }

  private func save(_ updated: DesktopContact) {
    do {
      try access.saveContact(updated)
      saveError = nil
    } catch { saveError = error.localizedDescription }
  }
}

/// Edits a contact's name and labels; adding one goes through AddContactSheet.
/// The email is the one they were added with: the directory finds a key from
/// an email, never an email from a key, so it is not editable here.
private struct ContactEditor: View {
  @State private var contact: DesktopContact
  @ObservedObject var access: AccessModel
  @State private var error: String?
  @FocusState private var nameFocused: Bool
  @Environment(\.dismiss) private var dismiss

  init(contact: DesktopContact, access: AccessModel) {
    _contact = State(initialValue: contact)
    self.access = access
  }

  private var trimmedName: String { contact.name.trimmingCharacters(in: .whitespacesAndNewlines) }

  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      HStack(spacing: 12) {
        Text(AccessText.initials(trimmedName.isEmpty ? "?" : trimmedName))
          .font(.system(size: 14, weight: .semibold))
          .frame(width: 40, height: 40).background(.quaternary, in: Circle())
        VStack(alignment: .leading, spacing: 2) {
          Text(trimmedName.isEmpty ? "Edit Contact" : trimmedName).font(.headline).lineLimit(1)
          if let email = contact.emailIdentity?.email {
            Text(email).font(.system(size: 12)).foregroundStyle(.secondary).lineLimit(1)
          }
          HStack(spacing: 4) {
            Text(contact.id).font(.system(size: 11, design: .monospaced))
              .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
            CopyButton(text: contact.id) { copied in
              Image(systemName: copied ? "checkmark" : "doc.on.doc").font(.system(size: 10))
            }.buttonStyle(.plain).foregroundStyle(.secondary).help("Copy key")
          }
        }
      }
      Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 12, verticalSpacing: 12) {
        GridRow {
          Text("Name").foregroundStyle(.secondary).gridColumnAlignment(.trailing)
          TextField("Name", text: $contact.name).textFieldStyle(.roundedBorder)
            .focused($nameFocused).accessibilityLabel("Contact name").onSubmit(save)
        }
        GridRow {
          Text("Labels").foregroundStyle(.secondary)
          ContactLabelsField(labels: $contact.labels)
        }
      }
      if let error {
        Text(error).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
      }
      HStack {
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
        Button("Save", action: save).keyboardShortcut(.defaultAction)
          .disabled(trimmedName.isEmpty)
      }
    }.padding(22).frame(width: 460)
      .onAppear { nameFocused = true }
      .onChange(of: access.selfDid) { _, _ in dismiss() }
  }

  private func save() {
    guard !trimmedName.isEmpty else { return }
    var updated = contact
    updated.name = trimmedName
    do {
      try access.saveContact(updated)
      dismiss()
    } catch { self.error = error.localizedDescription }
  }
}

/// Picks one of my spaces and copies a link only this contact's account can
/// claim. The link then appears in that space's Links as "Link for <name>".
private struct ContactInviteSheet: View {
  let contact: DesktopContact
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  @State private var spaceID = ""
  @State private var inviting = false
  @State private var copiedTo: String?
  @State private var error: String?
  @Environment(\.dismiss) private var dismiss
  /// Spaces the contact has not already joined, as far as the rosters show.
  private var candidates: [TonkSpace] {
    spaces.filter { space in
      access.spaces[space.id]?.members.contains { $0.did == contact.id } != true
    }
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Invite \(contact.name)").font(.headline)
      if candidates.isEmpty {
        Text("\(contact.name) is already in all of your listed spaces.")
          .foregroundStyle(.secondary)
      } else {
        Picker("Space", selection: $spaceID) {
          Text("Choose a space").tag("")
          ForEach(candidates) { space in Text(space.title).tag(space.id) }
        }.disabled(inviting)
      }
      Text(
        "Only \(contact.name)’s Tonk account can use this link. It gives read and write access to the whole space."
      )
      .font(.caption).foregroundStyle(.secondary).fixedSize(horizontal: false, vertical: true)
      if let copiedTo {
        Text(
          "Link copied. Send it to \(contact.name); you can copy or revoke it later in \(copiedTo)’s links."
        )
        .fixedSize(horizontal: false, vertical: true)
      }
      if let error {
        Text(error).foregroundStyle(.red).fixedSize(horizontal: false, vertical: true)
      }
      HStack {
        Spacer()
        Button(copiedTo == nil ? "Cancel" : "Done") { dismiss() }.keyboardShortcut(.cancelAction)
        if copiedTo == nil {
          Button(inviting ? "Making link…" : "Copy invite link") {
            guard let space = candidates.first(where: { $0.id == spaceID }) else { return }
            inviting = true
            error = nil
            Task {
              defer { inviting = false }
              if await access.inviteContact(contact, to: space) {
                copiedTo = space.title
              } else {
                error = access.notice ?? "Couldn’t make the link."
              }
            }
          }.keyboardShortcut(.defaultAction).disabled(inviting || spaceID.isEmpty)
        }
      }
    }.padding(22).frame(width: 400)
      .onChange(of: access.selfDid) { _, _ in dismiss() }
  }
}

/// Wrap labels without clipping long or user-defined names in the narrow detail pane.
private struct ContactLabelLayout: Layout {
  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    arrange(subviews, width: proposal.width ?? 300).size
  }
  func placeSubviews(
    in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()
  ) {
    let layout = arrange(subviews, width: bounds.width)
    for (index, point) in layout.points.enumerated() {
      subviews[index].place(
        at: CGPoint(x: bounds.minX + point.x, y: bounds.minY + point.y),
        proposal: ProposedViewSize(
          width: min(subviews[index].sizeThatFits(.unspecified).width, bounds.width), height: 24))
    }
  }
  private func arrange(_ views: Subviews, width: CGFloat) -> (size: CGSize, points: [CGPoint]) {
    var x: CGFloat = 0
    var y: CGFloat = 0
    var points: [CGPoint] = []
    for view in views {
      let size = view.sizeThatFits(.unspecified)
      let itemWidth = min(size.width, width)
      if x > 0 && x + itemWidth > width {
        x = 0
        y += 30
      }
      points.append(CGPoint(x: x, y: y))
      x += itemWidth + 6
    }
    return (CGSize(width: width, height: views.isEmpty ? 0 : y + 24), points)
  }
}
