import HarnessCore
import SwiftUI

struct ChatHistoryView: View {
  @ObservedObject var model: HarnessModel
  let space: TonkSpace?
  let onOpen: (ChatSession) -> Void
  @Environment(\.dismiss) private var dismiss
  @State private var search = ""
  private var allSessions: [ChatSession] {
    return model.saved.chats(in: space?.id).filter {
      !$0.conversation.messages.isEmpty || !($0.conversation.draft ?? "").isEmpty
        || ($0.id == model.saved.activeSessionID && model.showingChat)
    }
  }
  private var sessions: [ChatSession] {
    allSessions.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) }
  }
  var body: some View {
    VStack(alignment: .leading, spacing: 16) {
      Text("Chats in \(space?.title ?? "this space")")
        .font(.title2.weight(.semibold))
      if !allSessions.isEmpty {
        GlassSearchField(title: "Find a chat", text: $search)
      }
      Group {
        if sessions.isEmpty {
          VStack(spacing: 12) {
            Image(
              systemName: allSessions.isEmpty ? "bubble.left.and.bubble.right" : "magnifyingglass"
            )
            .font(.system(size: 32)).foregroundStyle(.tertiary)
            Text(allSessions.isEmpty ? "No chats yet" : "No matching chats")
              .font(.headline)
            Text(
              allSessions.isEmpty
                ? "Start a chat to ask questions or make changes in this space. Your chats will appear here."
                : "Try a different search."
            )
            .foregroundStyle(.secondary).multilineTextAlignment(.center).frame(maxWidth: 320)
          }.frame(maxWidth: .infinity, maxHeight: .infinity)
        } else {
          List(sessions) { session in
            Button {
              onOpen(session)
            } label: {
              VStack(alignment: .leading, spacing: 6) {
                Text(session.title).font(.headline).lineLimit(2).help(session.title)
                HStack {
                  Text(
                    session.conversation.resolvedModel ?? session.connection?.model
                      ?? session.provider.title)
                  Spacer()
                  Text(session.updatedAt, style: .date)
                }.font(.caption).foregroundStyle(.secondary)
              }.padding(.vertical, 6).contentShape(Rectangle())
            }.buttonStyle(.plain)
              .disabled(model.busy || model.connecting || model.creatingSpace || model.loginPending)
          }.listStyle(.inset)
        }
      }.frame(maxWidth: .infinity, maxHeight: .infinity)
      if let error = model.error { Text(error).font(.caption).foregroundStyle(.secondary) }
      HStack {
        Button("New chat", systemImage: "square.and.pencil") {
          model.startSpaceChat()
          dismiss()
        }
        .disabled(model.busy || model.connecting || model.creatingSpace || model.loginPending)
        Spacer()
        Button("Done") { dismiss() }.keyboardShortcut(.cancelAction)
      }
    }.padding(24).frame(width: 520, height: 440, alignment: .topLeading).nativeControl()

  }
}
