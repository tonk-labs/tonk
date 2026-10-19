import HarnessCore
import SwiftUI

/// Every link across spaces, or only agent links, grouped by space.
struct LinksPageView: View {
  let agentsOnly: Bool
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  var onNewLink: () -> Void

  var body: some View {
    let groups = spaces.compactMap { space -> (TonkSpace, [AccessLink])? in
      let rows = access.links(for: space.id).filter { !agentsOnly || $0.forAgents }
      return rows.isEmpty ? nil : (space, rows)
    }
    Form {
      Section {
      } footer: {
        Text(
          agentsOnly
            ? "Links agents use to read and change a space. Each lasts 90 days and can be revoked at any time."
            : "Every way into your spaces: plain links for people, links for agents, and public addresses."
        )
        .foregroundStyle(.secondary)
      }
      if let error = access.errors["account"] {
        Section {
          Text("Agent links could not be refreshed: " + error)
          Button("Try Again") { Task { await access.refreshAll(spaces) } }
        }
      }
      ForEach(spaces.filter { access.errors[$0.id] != nil }) { space in
        Section(space.title) {
          Text(access.errors[space.id] ?? "")
          Button("Try Again") { Task { await access.refresh(space) } }
        }
      }
      if groups.isEmpty {
        Section {
          HStack {
            Text(
              loadingAny
                ? "Loading links…"
                : (!access.errors.isEmpty
                  ? "Some links could not be loaded."
                  : (agentsOnly ? "No agent links yet." : "No links yet."))
            )
            .foregroundStyle(.secondary)
            Spacer()
            Button("New Link…", action: onNewLink).disabled(spaces.isEmpty)
          }
        }
      }
      ForEach(groups, id: \.0.id) { space, rows in
        Section(space.title) {
          ForEach(rows) { link in
            AccessLinkRow(link: link, space: space, access: access)
          }
        }
      }
    }
    .formStyle(.grouped)
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
    VStack(spacing: 0) {
      Form {
        Section {
          Picker("Space", selection: $subject) {
            ForEach(spaces) { Text($0.title).tag($0.id) }
          }
          Picker("For", selection: $kind) {
            Text("People").tag(AccessModel.NewLink.person)
            Text("Agents").tag(AccessModel.NewLink.agent)
            Text("Public").tag(AccessModel.NewLink.publicRead)
          }
          .pickerStyle(.segmented)
        } header: {
          Text("New Link")
        }
        Section {
          LabeledContent("Access", value: grant)
        } footer: {
          Text(explanation).foregroundStyle(.secondary)
        }
      }
      .formStyle(.grouped)
      .scrollDisabled(true)
      HStack {
        if creating { ProgressView().controlSize(.small) }
        Spacer()
        Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction).disabled(creating)
        Button(kind == .publicRead ? "Make Public and Copy" : "Make and Copy Link") {
          guard let space else { return }
          Task { if await access.make(kind, in: space) { dismiss() } }
        }
        .keyboardShortcut(.defaultAction)
        .disabled(creating || space == nil)
      }
      .padding([.horizontal, .bottom], 20)
    }
    .frame(width: 480)
    .fixedSize(horizontal: false, vertical: true)
    .interactiveDismissDisabled(creating)
  }

  private var grant: String {
    switch kind {
    case .person: "Anyone · Write · Never expires"
    case .agent: "Anyone with the link · Write · 90 days"
    case .publicRead: "Anyone · Read · Never expires"
    }
  }

  private var explanation: String {
    switch kind {
    case .person:
      "Anyone who opens it can join the whole space and edit it. Reset it later to stop new people joining."
    case .agent:
      "Paste it into Claude Code, Cursor or another MCP client. Keep it private; it works like a password."
    case .publicRead:
      "Anyone with the space’s address can read all of it. Only admins can make a space public."
    }
  }
}
