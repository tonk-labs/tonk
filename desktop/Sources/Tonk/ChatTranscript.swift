import HarnessCore

/// Group only for display; retain the original item IDs and messages in saved history.
enum ChatTranscript {
  static func grouped(_ messages: [ChatMessage]) -> [ChatMessage] {
    var groups: [ChatMessage] = []
    for message in messages {
      if message.role == "assistant", groups.last?.role == "assistant" {
        groups[groups.count - 1].text += "\n\n" + message.text
      } else {
        groups.append(message)
      }
    }
    return groups
  }
}
