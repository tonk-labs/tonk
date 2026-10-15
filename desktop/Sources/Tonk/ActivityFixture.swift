import AppKit
import HarnessCore

/// Explicit visual fixture. Never loads an account, a provider or existing app data.
@MainActor
enum ActivityFixture {
  static func prepare(model: HarnessModel, runtime: RuntimeModel) {
    let args = ProcessInfo.processInfo.arguments
    guard RuntimeLocation.deployment == .local, args.contains("--data-dir"),
      args.contains("--web-data-id"), model.saved.conversation.messages.isEmpty
    else {
      NSApp.terminate(nil)
      return
    }
    guard
      let space = try? JSONDecoder().decode(
        TonkSpace.self,
        from: Data(#"{"subject":"did:key:zFixture","name":"Activity layout fixture"}"#.utf8))
    else { return }
    model.saved.conversation.space = space
    model.saved.conversation.messages = [
      ChatMessage(id: "first", role: "user", text: "Create a checklist."),
      ChatMessage(
        id: "reply-one", role: "assistant", text: "Created three tasks and checked the result."),
      ChatMessage(id: "second", role: "user", text: "Check the filters."),
      ChatMessage(
        id: "reply-two", role: "assistant",
        text: "The read completed. The update could not be confirmed; query before retrying."),
    ]
    model.saved.conversation.appendToolActivity("Reading local space", messageID: "first")
    model.saved.conversation.appendToolActivity("Updating local space", messageID: "first")
    model.saved.conversation.appendToolActivity(
      "Inspecting attached space preview", messageID: "second")
    model.saved.conversation.appendToolActivity(
      "Evaluation could not be confirmed. Do not repeat it; query the space first.",
      messageID: "second")
    model.openedSpace = space
    model.showingChat = true
    model.runtime = runtime
    runtime.webView.loadHTMLString(
      "<p>Layout fixture only. No live worker or model.</p>", baseURL: nil)
  }
}
