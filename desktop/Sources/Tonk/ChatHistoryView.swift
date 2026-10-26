import HarnessCore
import SwiftUI

/// The chat sidebar's list of chats in one space. The sidebar's own toolbar
/// holds New chat and the title, so the list is only search and rows.
struct ChatHistoryView: View {
  @ObservedObject var model: HarnessModel
  let space: TonkSpace?
  let onOpen: (ChatSession) -> Void
  var onNewChat: () -> Void = {}
  @State private var search = ""
  private var allSessions: [ChatSession] {
    return model.saved.chats(in: space?.id).filter {
      !$0.conversation.messages.isEmpty || !($0.conversation.draft ?? "").isEmpty
    }
  }
  private var sessions: [ChatSession] {
    allSessions.filter { search.isEmpty || $0.title.localizedCaseInsensitiveContains(search) }
  }
  private var disabled: Bool {
    model.busy || model.connecting || model.creatingSpace || model.loginPending
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 6) {
      HStack(spacing: 6) {
        Image(systemName: "magnifyingglass").font(.system(size: 12)).foregroundStyle(.secondary)
          .accessibilityHidden(true)
        TextField("Search chats", text: $search).textFieldStyle(.plain)
          .accessibilityLabel("Search chats")
      }.font(.system(size: 13)).padding(.horizontal, 10).frame(height: 28)
        .background(.fill.tertiary, in: Capsule())
      ScrollView {
        LazyVStack(alignment: .leading, spacing: 2) {
          ForEach(ChatDateGroup.allCases, id: \.self) { group in
            let rows = sessions.filter { ChatDateGroup($0.updatedAt) == group }
            if !rows.isEmpty {
              Text(group.title).font(.system(size: 11, weight: .medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 8).padding(.top, 10).padding(.bottom, 2)
              ForEach(rows) { session in row(session, group: group) }
            }
          }
          if sessions.isEmpty {
            Text(allSessions.isEmpty ? "No chats yet" : "No matching chats")
              .font(.system(size: 13)).foregroundStyle(.secondary).padding(8)
            if allSessions.isEmpty {
              Button("New chat", action: onNewChat).nativeControl(prominent: true)
                .padding(.leading, 8).disabled(disabled)
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

  private func row(_ session: ChatSession, group: ChatDateGroup) -> some View {
    Button {
      onOpen(session)
    } label: {
      HStack {
        Text(session.title).lineLimit(1)
        Spacer(minLength: 8)
        Text(group.time(session.updatedAt, now: Date())).font(.system(size: 11))
          .foregroundStyle(.secondary).monospacedDigit()
      }.font(.system(size: 13)).padding(.horizontal, 8).frame(height: 30)
        .contentShape(Rectangle())
    }.buttonStyle(.plain)
      .help(session.title)
      .background(
        session.id == model.saved.activeSessionID ? Color.primary.opacity(0.07) : .clear,
        in: RoundedRectangle(cornerRadius: 6)
      )
      .disabled(disabled)
  }
}

/// Chats are grouped by day, and each group shows the time the way that suits it:
/// minutes for today, a clock time for yesterday, a weekday within the week.
enum ChatDateGroup: CaseIterable {
  case today, yesterday, previousWeek, older

  init(_ date: Date, now: Date = Date(), calendar: Calendar = .current) {
    if calendar.isDate(date, inSameDayAs: now) {
      self = .today
    } else if let yesterday = calendar.date(byAdding: .day, value: -1, to: now),
      calendar.isDate(date, inSameDayAs: yesterday)
    {
      self = .yesterday
    } else if let weekAgo = calendar.date(
      byAdding: .day, value: -7, to: calendar.startOfDay(for: now)), date >= weekAgo
    {
      self = .previousWeek
    } else {
      self = .older
    }
  }

  var title: String {
    switch self {
    case .today: "Today"
    case .yesterday: "Yesterday"
    case .previousWeek: "Previous 7 days"
    case .older: "Older"
    }
  }

  func time(_ date: Date, now: Date) -> String {
    switch self {
    case .today:
      let minutes = max(0, Int(now.timeIntervalSince(date) / 60))
      if minutes < 1 { return "now" }
      if minutes < 60 { return "\(minutes)m" }
      return "\(minutes / 60)h"
    case .yesterday:
      return date.formatted(date: .omitted, time: .shortened)
    case .previousWeek:
      return date.formatted(.dateTime.weekday(.abbreviated))
    case .older:
      return date.formatted(.dateTime.day().month(.abbreviated))
    }
  }
}
