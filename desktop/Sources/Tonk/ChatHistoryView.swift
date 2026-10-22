import HarnessCore
import SwiftUI

struct ChatHistoryView: View {
  @ObservedObject var model: HarnessModel
  let space: TonkSpace?
  let onOpen: (ChatSession) -> Void
  var inline = false
  /// Shows the chat pane after New chat; the list is the sidebar's root.
  var onShowChat: () -> Void = {}
  @Environment(\.dismiss) private var dismiss
  @State private var search = ""
  private var allSessions: [ChatSession] {
    return model.saved.chats(in: space?.id).filter {
      !$0.conversation.messages.isEmpty || !($0.conversation.draft ?? "").isEmpty
    }
  }
  private var sessions: [ChatSession] {
    allSessions.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) }
  }
  var body: some View {
    if inline { inlineHistory } else { sheetHistory }
  }
  private var sheetHistory: some View {
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
  private var inlineHistory: some View {
    VStack(alignment: .leading, spacing: 10) {
      HStack(spacing: 4) {
        Text("Chats").font(.system(size: 13, weight: .semibold))
        Spacer()
        ViewChromeButton(title: "New Chat", asset: "r2IconSquarePen", action: startChat)
          .disabled(model.busy || model.connecting || model.creatingSpace || model.loginPending)
      }.padding(.leading, 6).padding(.top, 2)
      Text("In \(space?.title ?? "this space"), with \(model.agentName)")
        .font(.system(size: 12)).foregroundStyle(.secondary).padding(.leading, 6)
      HStack(spacing: 6) {
        Image(systemName: "magnifyingglass").font(.system(size: 12)).foregroundStyle(.secondary)
          .accessibilityHidden(true)
        TextField("Search chats", text: $search).textFieldStyle(.plain)
          .accessibilityLabel("Search chats")
      }.font(.system(size: 13)).padding(.horizontal, 10).frame(height: 28)
        .background(.fill.tertiary, in: Capsule())
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 2) {
          ForEach(["Today", "Yesterday", "Previous 7 days", "Older"], id: \.self) { group in
            let rows = sessions.filter { dateGroup($0.updatedAt) == group }
            if !rows.isEmpty {
              Text(group).font(.system(size: 11)).foregroundStyle(.secondary)
                .padding(.horizontal, 8).padding(.top, 10).padding(.bottom, 4)
              ForEach(rows) { session in
                Button {
                  onOpen(session)
                } label: {
                  HStack {
                    Text(session.title).lineLimit(1)
                    Spacer(minLength: 8)
                    Text(AccessText.relative(session.updatedAt, now: Date())).font(
                      .system(size: 11)
                    )
                    .foregroundStyle(
                      .secondary)
                  }.font(.system(size: 13)).padding(.horizontal, 8).frame(height: 30)
                    .contentShape(Rectangle())
                }.buttonStyle(.plain)
                  .background(
                    session.id == model.saved.activeSessionID
                      ? Color.primary.opacity(0.06) : .clear,
                    in: RoundedRectangle(cornerRadius: 6)
                  )
                  .disabled(
                    model.busy || model.connecting || model.creatingSpace || model.loginPending)
              }
            }
          }
          if sessions.isEmpty {
            Text(allSessions.isEmpty ? "No chats yet" : "No matching chats")
              .font(.system(size: 13)).foregroundStyle(.secondary).padding(8)
            if allSessions.isEmpty {
              Button("New chat", action: startChat).nativeControl(prominent: true).padding(
                .leading, 8
              )
              .disabled(
                model.busy || model.connecting || model.creatingSpace || model.loginPending)
            }
          }
        }
      }
      if let error = model.error { Text(error).font(.caption).foregroundStyle(.secondary) }
    }.frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
      // The live chat may not be in the saved list yet; record it before listing.
      .onAppear {
        if !model.saved.conversation.messages.isEmpty { model.saved.checkpointChat() }
      }
  }

  private func startChat() {
    model.startSpaceChat()
    onShowChat()
  }

  private func dateGroup(_ date: Date) -> String {
    let calendar = Calendar.current
    if calendar.isDateInToday(date) { return "Today" }
    if calendar.isDateInYesterday(date) { return "Yesterday" }
    if let weekAgo = calendar.date(byAdding: .day, value: -7, to: calendar.startOfDay(for: Date())),
      date >= weekAgo
    {
      return "Previous 7 days"
    }
    return "Older"
  }

}
