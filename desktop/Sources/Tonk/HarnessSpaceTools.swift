import Foundation
import HarnessCore

@MainActor
extension HarnessModel {
  var spaceInstructions: String {
    guard let space = saved.conversation.space else {
      return TonkAgentContext.instructions + "\n\n" + """
        No Tonk space is attached. When the user wants a durable artifact or workspace,
        use tonk_propose_space with a short name and reason. This displays a native
        approval card, not a created space. Stop after proposing and wait for acceptance.
        tonk_guide is available now. Other space tools require an attached space.
        """
    }
    return TonkAgentContext.instructions + "\n\n" + """
      Attached Tonk space: \(space.subject). Calls are pinned to this space; omit a space argument.
      Replies appear in a narrow sidebar: lead with the result, then use short lists.
      Use tonk_inspect_view to inspect the open preview after authoring. It reports
      text, controls and errors, not screenshots or proof of interaction.
      """
  }

  func attachSpace(_ space: TonkSpace) {
    guard !busy, !creatingSpace else { return }
    newConversation()
    // newConversation preserves the old conversation if archival fails.
    guard saved.conversation.threadID == nil, saved.conversation.messages.isEmpty else { return }
    saved.conversation.space = space
    persist()
  }

  func callSpaceTool(_ params: JSONValue) async -> JSONValue {
    do {
      guard busy, params["threadId"].string == saved.conversation.threadID,
        let requestTurn = params["turnId"].string, requestTurn == activeTurnID,
        params["namespace"] == .null,
        let tool = params["tool"].string
      else { throw HarnessError.message("No matching active turn.") }
      if tool == "tonk_guide" {
        let result = try TonkAgentContext.guide(params["arguments"])
        return SpaceTools.response(
          String(decoding: try JSONEncoder().encode(result), as: UTF8.self), success: true)
      }
      if tool == "tonk_propose_space" {
        guard saved.conversation.space == nil, !creatingSpace else {
          throw HarnessError.message("This conversation already has a space or is creating one.")
        }
        let proposal = try SpaceProposal(arguments: params["arguments"])
        guard saved.conversation.spaceProposal == nil else {
          return SpaceTools.response(
            "A proposal is already waiting for the user. Stop and wait.", success: true)
        }
        var next = saved
        next.conversation.spaceProposal = proposal
        try store.save(next)
        saved = next
        spaceCreationError = nil
        return SpaceTools.response(
          "Proposal shown. No space has been created. Stop and wait for the user to accept or dismiss the native card.",
          success: true)
      }
      guard let space = saved.conversation.space, let runtime else {
        throw HarnessError.message(
          "No space is attached. Propose one and wait for acceptance first.")
      }
      if TonkToolCatalog.primitiveNames.contains(tool) {
        activity =
          tool == "tonk_evaluate"
          ? "Updating local space"
          : (tool == "tonk_query" ? "Reading local space" : "Validating notation")
        toolActivity.append(activity)
        let result = try await runtime.performBuildTool(
          space, tool: tool, arguments: params["arguments"])
        let data = try JSONEncoder().encode(result)
        activity = "Thinking"
        return SpaceTools.response(String(decoding: data, as: UTF8.self), success: true)
      }
      _ = try SpaceTools.validate(tool: tool, arguments: params["arguments"])
      try Task.checkCancellation()
      if tool == "tonk_inspect_view" {
        activity = "Inspecting space preview"
        toolActivity.append("Inspecting attached space preview")
        let text = try await runtime.inspectView(space)
        activity = "Thinking"
        return SpaceTools.response(text, success: true)
      }
      throw HarnessError.message("Unavailable tool. No saved operation was replayed.")
    } catch {
      let detail =
        (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
        ?? error.localizedDescription
      toolActivity.append("Space tool failed: \(detail)")
      return SpaceTools.response(detail, success: false)
    }
  }
}
