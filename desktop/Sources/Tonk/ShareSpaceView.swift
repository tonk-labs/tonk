import AppKit
import HarnessCore
import SwiftUI

struct ShareSpaceView: View {
  @ObservedObject var runtime: RuntimeModel
  let space: TonkSpace
  @Environment(\.dismiss) private var dismiss
  @State private var recipient = ShareRecipient.person
  @StateObject private var sharing = ShareInvitations()
  @State private var showingCopied = false
  @State private var copyFeedbackID = 0

  private var invitation: ShareInvitation { sharing[recipient] }

  private var creating: Bool { sharing.creating }

  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Share “\(space.title)”").font(.title2.weight(.semibold))
      Picker("Share with", selection: $recipient) {
        Text("Person").tag(ShareRecipient.person)
        Text("Agent").tag(ShareRecipient.agent)
      }
      .pickerStyle(.segmented).disabled(creating)
      Text(
        recipient == .person
          ? "Anyone with this link can join and collaborate in this space."
          : "Give an AI agent access to read and change this space. Keep the link private."
      )
      .foregroundStyle(.secondary)
      .frame(maxWidth: .infinity, minHeight: 40, alignment: .topLeading)
      if invitation.failed {
        Text(
          "Couldn’t confirm the link. Check your connection and sharing settings in Tonk before creating another."
        )
        .foregroundStyle(.secondary)
      }
      GlassControls {
        HStack(spacing: 8) {
          Spacer()
          Button("Done") { dismiss() }.keyboardShortcut(.cancelAction).disabled(creating)
          Button {
            let target = recipient
            Task {
              let copied = await sharing.copyLink(
                for: target,
                personLink: { try await runtime.createShareLink(space).absoluteString },
                agentLink: { try await runtime.createCLILink(space) },
                copy: { link in
                  NSPasteboard.general.clearContents()
                  return NSPasteboard.general.setString(link, forType: .string)
                })
              if copied {
                showingCopied = true
                copyFeedbackID += 1
              }
            }
          } label: {
            HStack(spacing: 8) {
              if sharing.showingProgress {
                ProgressView().controlSize(.small)
                Text("Copying…")
              } else if showingCopied {
                Label("Copied", systemImage: "checkmark")
              } else {
                Text(
                  invitation.copied
                    ? "Copy again"
                    : (recipient == .person ? "Copy invitation" : "Copy agent link")
                )
              }
            }.frame(minWidth: 120)
          }
          .nativeControl(prominent: true)
          .keyboardShortcut(.defaultAction)
          .disabled(sharing.showingProgress || invitation.failed)
        }.controlSize(.large)
      }
    }
    .padding(24).frame(width: 420).nativeControl()
    .interactiveDismissDisabled(creating)
    .onChange(of: recipient) { _, _ in showingCopied = false }
    .task(id: copyFeedbackID) {
      guard copyFeedbackID > 0 else { return }
      do { try await Task.sleep(for: .seconds(2)) } catch { return }
      showingCopied = false
    }
  }

}
