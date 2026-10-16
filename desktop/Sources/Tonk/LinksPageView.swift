import HarnessCore
import SwiftUI

/// Every link across spaces, or only agent links, grouped by space.
struct LinksPageView: View {
  let agentsOnly: Bool
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  var onNewLink: () -> Void

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 28) {
        Text(
          agentsOnly
            ? "Links agents use to read and change a space. Each lasts 90 days and can be revoked at any time."
            : "Every way into your spaces: plain links for people, agent links and public addresses."
        )
        .foregroundStyle(.secondary)
        let groups = spaces.compactMap { space -> (TonkSpace, [AccessLink])? in
          let rows = access.links(for: space.id).filter { !agentsOnly || $0.forAgents }
          return rows.isEmpty ? nil : (space, rows)
        }
        if groups.isEmpty {
          OverviewCard {
            HStack {
              Text(
                loadingAny
                  ? "Loading links…"
                  : (agentsOnly ? "No agent links yet." : "No links yet.")
              )
              .foregroundStyle(.secondary)
              Spacer()
              Button("New link", action: onNewLink).disabled(spaces.isEmpty)
            }
            .padding(20)
          }
        }
        ForEach(groups, id: \.0.id) { space, rows in
          VStack(alignment: .leading, spacing: 12) {
            SectionHeading(title: space.title)
            OverviewCard {
              ForEach(Array(rows.enumerated()), id: \.element.id) { index, link in
                if index > 0 { Divider() }
                AccessLinkRow(link: link, space: space, access: access)
              }
            }
          }
        }
      }
      .frame(maxWidth: 820, alignment: .leading)
      .padding(.horizontal, 40).padding(.vertical, 32)
      .frame(maxWidth: .infinity)
    }
  }

  private var loadingAny: Bool {
    !access.loading.isEmpty || !access.loadedAll
  }
}

/// Choose a space and who the link is for, then make and copy it.
struct NewLinkSheet: View {
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  @State var subject: String
  @State private var kind = AccessModel.NewLink.person
  @Environment(\.dismiss) private var dismiss

  private var space: TonkSpace? { spaces.first(where: { $0.id == subject }) }
  private var creating: Bool { access.working.contains("new:" + subject) }

  var body: some View {
    VStack(alignment: .leading, spacing: 18) {
      Text("New link").font(.title2.weight(.semibold))
      Picker("Space", selection: $subject) {
        ForEach(spaces) { Text($0.title).tag($0.id) }
      }
      Picker("For", selection: $kind) {
        Text("People").tag(AccessModel.NewLink.person)
        Text("Agents").tag(AccessModel.NewLink.agent)
        Text("Public").tag(AccessModel.NewLink.publicRead)
      }
      .pickerStyle(.segmented)
      VStack(alignment: .leading, spacing: 6) {
        Text(grant).font(.headline)
        Text(explanation).foregroundStyle(.secondary)
          .fixedSize(horizontal: false, vertical: true)
      }
      .frame(maxWidth: .infinity, minHeight: 72, alignment: .topLeading)
      HStack {
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction).disabled(creating)
        Button {
          guard let space else { return }
          Task { if await access.make(kind, in: space) { dismiss() } }
        } label: {
          HStack(spacing: 8) {
            if creating { ProgressView().controlSize(.small) }
            Text(kind == .publicRead ? "Make public and copy" : "Make and copy link")
          }
        }
        .buttonStyle(.borderedProminent).tint(.tonkInk)
        .keyboardShortcut(.defaultAction)
        .disabled(creating || space == nil)
      }
    }
    .padding(24).frame(width: 460)
    .interactiveDismissDisabled(creating)
  }

  private var grant: String {
    switch kind {
    case .person: "Anyone · Write · Whole space · Never expires"
    case .agent: "Anyone with the link · Write · Whole space · Expires in 90 days"
    case .publicRead: "Anyone · Read · Whole space · Never expires"
    }
  }

  private var explanation: String {
    switch kind {
    case .person:
      "Anyone who opens it can join and edit. Reset it later to stop new people joining."
    case .agent:
      "Paste it into Claude Code, Cursor or another MCP client. Keep it private; it works like a password."
    case .publicRead:
      "Anyone with the space’s address can read it. Only admins can make a space public."
    }
  }
}
