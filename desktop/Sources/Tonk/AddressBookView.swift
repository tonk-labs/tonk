import HarnessCore
import SwiftUI

struct DesktopContact: Codable, Identifiable, Equatable {
  var id: String
  var name: String
  var labels: [String]
}

struct AddressBookView: View {
  @ObservedObject var access: AccessModel
  let spaces: [TonkSpace]
  @State private var search = ""
  @State private var selected: String?
  @State private var editing: DesktopContact?
  @State private var saveError: String?
  private var contacts: [DesktopContact] {
    access.contacts.filter {
      search.isEmpty || ($0.name + $0.id).localizedCaseInsensitiveContains(search)
    }
  }
  var body: some View {
    VStack(spacing: 0) {
      HSplitView {
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
        }.listStyle(.plain).scrollContentBackground(.hidden).frame(
          minWidth: 180, idealWidth: 220, maxWidth: 230)
        if let contact = contacts.first(where: { $0.id == selected }) {
          ScrollView {
            VStack(alignment: .leading, spacing: 20) {
              HStack(spacing: 12) {
                Text(AccessText.initials(contact.name)).font(.system(size: 13, weight: .semibold))
                  .frame(width: 40, height: 40).background(.quaternary, in: Circle())
                VStack(alignment: .leading, spacing: 4) {
                  Text(contact.name).font(.system(size: 15, weight: .semibold))
                  Text("Known identity").font(.system(size: 12)).foregroundStyle(.secondary)
                }
              }
              VStack(alignment: .leading, spacing: 8) {
                Text("did:key").font(.caption).foregroundStyle(.secondary)
                HStack(alignment: .top) {
                  Text(contact.id).font(.system(size: 12, design: .monospaced)).textSelection(
                    .enabled)
                  Spacer(minLength: 4)
                  Button("Copy") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(contact.id, forType: .string)
                  }
                }.padding(12).background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
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
                  Button("+ Add Label") { editing = contact }.buttonStyle(.plain)
                    .padding(.horizontal, 9).frame(height: 24)
                    .overlay(
                      Capsule().strokeBorder(.tertiary, style: StrokeStyle(lineWidth: 1, dash: [3]))
                    )
                }.font(.system(size: 12))
                Text("Suggested: Team · Work · Family · Verified").font(.system(size: 12))
                  .foregroundStyle(.secondary)
                Text("Labels are personal notes, not verified account types.").font(
                  .system(size: 11)
                ).foregroundStyle(.secondary)
                Button("Edit Contact…") { editing = contact }
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
              }
            }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
          }.frame(minWidth: 220)
        } else {
          Text(contacts.isEmpty ? "No contacts yet" : "Select a contact")
            .foregroundStyle(.secondary).frame(maxWidth: .infinity, maxHeight: .infinity)
        }
      }
    }.font(.system(size: 13))
      .toolbar {
        ToolbarItemGroup(placement: .primaryAction) {
          TextField("Name or key", text: $search).textFieldStyle(.roundedBorder).frame(width: 170)
          Button("Add Contact", systemImage: "plus") {
            editing = DesktopContact(id: "", name: "", labels: [])
          }.labelStyle(.iconOnly)
        }
      }
      .sheet(item: $editing) { contact in ContactEditor(contact: contact, access: access) }
      .onChange(of: contacts.map(\.id), initial: true) { _, ids in
        if selected == nil || !ids.contains(selected!) { selected = ids.first }
      }
  }
  private func copy(_ text: String) {
    NSPasteboard.general.clearContents()
    NSPasteboard.general.setString(text, forType: .string)
  }

}

private struct ContactEditor: View {
  @State var contact: DesktopContact
  @ObservedObject var access: AccessModel
  @State private var labels = ""
  @State private var error: String?
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Save Contact").font(.headline)
      TextField("Name", text: $contact.name)
      TextField("did:key", text: $contact.id)
      TextField("Labels, separated by commas", text: $labels)
      Text(
        "Contacts and labels are saved on this Mac. Saving a contact does not grant access or establish an agent identity."
      )
      .font(.caption).foregroundStyle(.secondary)
      if let error { Text(error).foregroundStyle(.red) }
      HStack {
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
        Button("Save") {
          contact.labels = labels.split(separator: ",").map {
            $0.trimmingCharacters(in: .whitespaces)
          }.filter { !$0.isEmpty }
          do {
            try access.saveContact(contact)
            dismiss()
          } catch { self.error = error.localizedDescription }
        }.keyboardShortcut(.defaultAction)
          .disabled(
            !contact.id.hasPrefix("did:key:") || contact.id.count <= 8
              || contact.name.trimmingCharacters(in: .whitespaces).isEmpty)
      }
    }.textFieldStyle(.roundedBorder).padding(22).frame(width: 400)
      .onAppear { labels = contact.labels.joined(separator: ", ") }
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
