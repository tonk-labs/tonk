import HarnessCore
import SwiftUI

struct ConnectionsPageView: View {
  let spaces: [TonkSpace]
  @ObservedObject var access: AccessModel
  @ObservedObject var model: HarnessModel
  var onSettings: () -> Void
  @State private var onboarding = false

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: 24) {
        VStack(alignment: .leading, spacing: 8) {
          Text("Chat agent").font(.system(size: 15, weight: .semibold))
          Text("Edits as you, only in Tonk’s chat beside a view, while you’re chatting.")
            .font(.system(size: 12)).foregroundStyle(.secondary)
          HStack(spacing: 12) {
            DesktopGlyph(name: "imgIconMessages", size: 16).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 3) {
              Text(model.provider.title)
              Text(model.modelLabel).font(.system(size: 12)).foregroundStyle(.secondary)
              Text(
                model.connecting
                  ? "Connecting…"
                  : (model.connected ? "Connected to Tonk chat" : "Not connected to Tonk chat")
              )
              .font(.system(size: 12)).foregroundStyle(.secondary)
            }
            Spacer()
            Button("Settings", action: onSettings)
          }.padding(14).background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
        }
        VStack(alignment: .leading, spacing: 8) {
          HStack {
            Text("Agent accounts").font(.system(size: 15, weight: .semibold))
            Spacer()
            Button("Add an agent") { onboarding = true }
          }
          Text(
            "Edit as themselves, under their own name and key, from wherever they run, until you remove them."
          )
          .font(.system(size: 12)).foregroundStyle(.secondary)
          Text("Account provisioning is not available in this build.")
            .foregroundStyle(.secondary).padding(14).frame(maxWidth: .infinity, alignment: .leading)
            .background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
        }
        VStack(alignment: .leading, spacing: 8) {
          Text("Connected agents").font(.system(size: 15, weight: .semibold))
          Text(
            "Agents, MCP clients and the CLI that connected through access you granted. Share links stay in All links."
          )
          .font(.system(size: 12)).foregroundStyle(.secondary)

        }
        if let error = access.errors["account"] {
          Text(error).foregroundStyle(.secondary)
          Button("Try again") { Task { await access.refreshAccount() } }
        }
        ForEach(spaces) { space in
          let grants = access.connections.filter { $0.subject == space.subject }
          if !grants.isEmpty {
            VStack(alignment: .leading, spacing: 8) {
              HStack {
                Text(space.title).fontWeight(.semibold)
                Spacer()
                Text("\(grants.count) connected")
                  .foregroundStyle(.secondary)
              }
              VStack(spacing: 0) {
                ForEach(grants, id: \.id) { grant in
                  if grant.id != grants.first?.id { Divider() }
                  ConnectionRow(agent: grant, space: space, access: access)
                }
              }.background(DesktopStyle.group, in: RoundedRectangle(cornerRadius: 10))
            }
          }
        }
        if access.connections.isEmpty && access.errors["account"] == nil {
          Text("No agents have connected yet.").foregroundStyle(.secondary)
        }
      }.padding(20)
    }.font(.system(size: 13)).controlSize(.small)
      // Flat buttons: these are row actions, not window chrome.
      .buttonStyle(.bordered).buttonBorderShape(.capsule)
      .sheet(isPresented: $onboarding) {
        VStack(alignment: .leading, spacing: 16) {
          Text("Add an agent").font(.system(size: 15, weight: .semibold))
          Text(
            "This creates a separate identity with its own name and key, sponsored by you. Account provisioning is not available in this build."
          )
          .foregroundStyle(.secondary)
          Text(
            "An MCP connection, a link pasted into a chat, or CLI authorization can provide access without creating that account. Agents that already connected appear under Connected agents."
          )
          .font(.system(size: 12)).foregroundStyle(.secondary)
          HStack {
            Spacer()
            Button("Done") { onboarding = false }.keyboardShortcut(.defaultAction)
          }
        }.padding(22).frame(width: 460)
      }
  }
}

private struct ConnectionRow: View {
  let agent: AgentConnection
  let space: TonkSpace
  @ObservedObject var access: AccessModel
  @State private var disconnecting = false
  var body: some View {
    HStack(spacing: 12) {
      DesktopGlyph(name: "imgIconPlug", size: 16).foregroundStyle(.secondary)
      VStack(alignment: .leading, spacing: 3) {
        Text(
          agent.label.isEmpty ? (agent.installations.first?.name ?? "Access grant") : agent.label)
        Text(
          agent.isTerminalAuthorization
            ? "CLI authorization · Read and write" : "Access grant · Read and write"
        )
        .font(.system(size: 12)).foregroundStyle(.secondary)
        if !agent.isTerminalAuthorization {
          Text("Connection method not reported").font(.system(size: 12)).foregroundStyle(.secondary)
        }
        Text(
          agent.installations.isEmpty
            ? "No registered clients"
            : "Registered clients: " + agent.installations.map(\.name).joined(separator: " · ")
        )
        .font(.system(size: 12)).foregroundStyle(.secondary)
        if let recipient = agent.recipient {
          Text("Recipient: " + recipient).font(.system(size: 11, design: .monospaced))
            .lineLimit(1).truncationMode(.middle).textSelection(.enabled).help(recipient)
        }
      }
      Spacer(minLength: 8)
      Button("Revoke access…") { disconnecting = true }
        .disabled(access.working.contains("agent:" + agent.id))
    }.padding(14)
      .confirmationDialog("Revoke this access grant?", isPresented: $disconnecting) {
        Button("Revoke Access", role: .destructive) {
          let link = AccessLink(kind: .agent, subject: space.subject, target: agent.id, usedBy: nil)
          Task { await access.revoke(link, space: space) }
        }
      } message: {
        Text(
          "Every client using this grant will lose access to \(space.title). This does not delete an agent account or change other grants."
        )
      }
  }
}
