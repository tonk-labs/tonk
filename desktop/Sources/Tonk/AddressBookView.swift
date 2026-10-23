import HarnessCore
import SwiftUI

struct DesktopContact: Codable, Identifiable, Equatable {
  var id: String
  var name: String
  var labels: [String]
  var emailIdentity: EmailContactIdentity? = nil
}

struct AddressBookView: View {
  @ObservedObject var access: AccessModel
  let spaces: [TonkSpace]
  var searchFocusRequest = 0
  @State private var search = ""
  @State private var selected: String?
  @State private var editing: DesktopContact?
  @State private var inviting: DesktopContact?
  @State private var adding = false
  @State private var saveError: String?
  @State private var addingLabel = false
  @State private var newLabel = ""
  @FocusState private var labelFocused: Bool
  private var contacts: [DesktopContact] {
    access.contacts.filter {
      search.isEmpty
        || ($0.name + $0.id + ($0.emailIdentity?.email ?? "")).localizedCaseInsensitiveContains(
          search)
    }
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
          }.nativeControl(circular: true).controlSize(.large).help("Add contact")
        }.padding(.horizontal, 12).padding(.top, 12).padding(.bottom, 4)
        List(contacts, selection: $selected) { contact in
          HStack(spacing: 10) {
            Text(AccessText.initials(contact.name)).font(.system(size: 10, weight: .semibold))
              .frame(width: 24, height: 24).background(.quaternary, in: Circle())
            VStack(alignment: .leading, spacing: 3) {
              Text(contact.name).lineLimit(1)
              HStack(spacing: 4) {
                Text(contact.id).font(.system(size: 11, design: .monospaced))
                  .foregroundStyle(.secondary).lineLimit(1).truncationMode(.middle)
                Button {
                  copy(contact.id)
                } label: {
                  DesktopGlyph(name: "r2CopyKey", size: 12)
                }
                .buttonStyle(.plain).help("Copy key").accessibilityLabel(
                  "Copy key for \(contact.name)")
              }
            }
          }.padding(.vertical, 4).tag(contact.id).listRowSeparator(.hidden)
            .contextMenu {
              Button("Edit Contact…") { editing = contact }
              if contact.id != access.selfDid {
                Button("Invite to a space…") { inviting = contact }.disabled(spaces.isEmpty)
              }
            }
        }
        // The sidebar style insets the selection as a rounded rectangle, as in frame 11.
        .listStyle(.sidebar).scrollContentBackground(.hidden)
      }.frame(width: 228)
      Divider()
      if let contact = contacts.first(where: { $0.id == selected }) {
        ScrollView {
          VStack(alignment: .leading, spacing: 22) {
            HStack(spacing: 12) {
              Text(AccessText.initials(contact.name)).font(.system(size: 13, weight: .semibold))
                .frame(width: 40, height: 40).background(.quaternary, in: Circle())
              VStack(alignment: .leading, spacing: 2) {
                Text(contact.name).font(.system(size: 15, weight: .semibold))
                Text("Known identity").font(.system(size: 12)).foregroundStyle(.secondary)
              }
            }
            VStack(alignment: .leading, spacing: 8) {
              Text("did:key").font(.caption).foregroundStyle(.secondary)
              HStack(alignment: .center) {
                Text(contact.id).font(.system(size: 12, design: .monospaced)).textSelection(
                  .enabled)
                Spacer(minLength: 4)
                Button("Copy") {
                  NSPasteboard.general.clearContents()
                  NSPasteboard.general.setString(contact.id, forType: .string)
                }
              }.padding(16).background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
            }
            if let identity = contact.emailIdentity {
              VStack(alignment: .leading, spacing: 6) {
                Text(identity.email).textSelection(.enabled)
                Text(
                  "Found through \(identity.directory.host ?? identity.directory.absoluteString) on \(identity.checkedAt.formatted(date: .abbreviated, time: .omitted))."
                )
                .font(.caption).foregroundStyle(.secondary)
              }
            }
            VStack(alignment: .leading, spacing: 8) {
              Text("Labels").font(.caption).foregroundStyle(.secondary)
              ContactLabelLayout {
                ForEach(Array(contact.labels.enumerated()), id: \.offset) { index, label in
                  Button {
                    var updated = contact
                    updated.labels.remove(at: index)
                    do {
                      try access.saveContact(updated)
                      saveError = nil
                    } catch { saveError = error.localizedDescription }
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
                    .onSubmit { addLabel(newLabel, to: contact) }
                    .onExitCommand { addingLabel = false }
                    .onChange(of: labelFocused) { _, focused in
                      if !focused { addLabel(newLabel, to: contact) }
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
              let suggestions = ContactLabelsField.suggestions.filter { suggestion in
                !contact.labels.contains { $0.caseInsensitiveCompare(suggestion) == .orderedSame }
              }
              if !suggestions.isEmpty {
                HStack(spacing: 4) {
                  Text("Suggested:")
                  ForEach(suggestions, id: \.self) { suggestion in
                    Button(suggestion) { addLabel(suggestion, to: contact) }.buttonStyle(.plain)
                      .help("Add the label \(suggestion)")
                    if suggestion != suggestions.last { Text("·") }
                  }
                }.font(.system(size: 12)).foregroundStyle(.secondary)
              }
              if let saveError { Text(saveError).foregroundStyle(.red) }
            }
            VStack(alignment: .leading, spacing: 8) {
              Text("Where it came from").font(.caption).foregroundStyle(.secondary)
              let shared = spaces.filter { space in
                access.spaces[space.id]?.members.contains { $0.did == contact.id } == true
              }
              Text(
                shared.isEmpty
                  ? "Saved on this Mac. No shared spaces are currently listed."
                  : "Seen in " + shared.map(\.title).joined(separator: " · ")
              )
              if contact.id != access.selfDid {
                Button("Invite to a space…") { inviting = contact }.disabled(spaces.isEmpty)
                  .help("Make a link only this contact’s account can use")
              }
            }
          }.frame(maxWidth: 680, alignment: .leading).padding(.horizontal, 20).padding(
            .vertical, 18
          )
          .frame(maxWidth: .infinity, alignment: .leading)
        }.frame(maxWidth: .infinity)
      } else {
        Text(
          !search.isEmpty && contacts.isEmpty
            ? "No matching contacts" : contacts.isEmpty ? "No contacts yet" : "Select a contact"
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
      .onChange(of: contacts.map(\.id), initial: true) { _, ids in
        if selected == nil || !ids.contains(selected!) { selected = ids.first }
      }
  }
  /// Labels are added in place; the full editor is for a contact's name and key.
  private func addLabel(_ label: String, to contact: DesktopContact) {
    let trimmed = label.trimmingCharacters(in: .whitespacesAndNewlines)
    addingLabel = false
    newLabel = ""
    guard !trimmed.isEmpty,
      !contact.labels.contains(where: { $0.caseInsensitiveCompare(trimmed) == .orderedSame })
    else { return }
    var updated = contact
    updated.labels.append(trimmed)
    do {
      try access.saveContact(updated)
      saveError = nil
    } catch { saveError = error.localizedDescription }
  }

  private func copy(_ text: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
  }

}

/// Edits a saved contact's name and labels; adding one goes through AddContactSheet.
private struct ContactEditor: View {
  @State var contact: DesktopContact
  @ObservedObject var access: AccessModel
  @State private var error: String?
  @Environment(\.dismiss) private var dismiss
  private var canSave: Bool {
    !contact.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Edit Contact").font(.headline)
      VStack(alignment: .leading, spacing: 4) {
        if let identity = contact.emailIdentity { Text(identity.email) }
        Text(contact.id).font(.system(size: 11, design: .monospaced)).foregroundStyle(.secondary)
          .lineLimit(1).truncationMode(.middle).textSelection(.enabled)
      }
      VStack(alignment: .leading, spacing: 6) {
        Text("Name").font(.caption).foregroundStyle(.secondary)
        TextField("Name", text: $contact.name).textFieldStyle(.roundedBorder)
          .accessibilityLabel("Contact name").onSubmit(save)
      }
      VStack(alignment: .leading, spacing: 6) {
        Text("Labels").font(.caption).foregroundStyle(.secondary)
        ContactLabelsField(labels: $contact.labels)
      }
      if let error { Text(error).foregroundStyle(.red) }
      HStack {
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
        Button("Save", action: save).keyboardShortcut(.defaultAction).disabled(!canSave)
      }
    }.padding(22).frame(width: 440)
      .onChange(of: access.selfDid) { _, _ in dismiss() }
  }

  private func save() {
    guard canSave else { return }
    var updated = contact
    updated.name = contact.name.trimmingCharacters(in: .whitespacesAndNewlines)
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
