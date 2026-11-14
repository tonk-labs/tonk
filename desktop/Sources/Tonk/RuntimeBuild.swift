import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  /// `page` is the window the calling chat belongs to, if any.
  func performBuildTool(
    _ space: TonkSpace, tool: String, arguments: JSONValue, page: SpacePage? = nil
  ) async throws -> JSONValue {
    if tool == "tonk_inspect_view" {
      _ = try SpaceTools.validate(tool: tool, arguments: arguments)
      let text = try await inspectView(space, in: page)
      return try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    }
    if TonkToolCatalog.primitiveNames.contains(tool) {
      try requireSpaceReady(space, page: page)
      try Task.checkCancellation()
      let input = String(decoding: try JSONEncoder().encode(arguments), as: UTF8.self)
      let result: [String: Any]
      do {
        result = try await accountScript(
          TonkAgentContext.workerScript,
          arguments: ["subject": space.subject, "tool": tool, "argumentsJSON": input])
      } catch {
        if tool == "tonk_evaluate" {
          throw HarnessError.message(
            "Evaluation could not be confirmed. Do not repeat it; query the space first.")
        }
        throw error
      }
      if let error = result["error"] as? String { throw HarnessError.message(error) }
      guard let response = result["response"] as? String else {
        throw HarnessError.message(
          tool == "tonk_evaluate"
            ? "Evaluation could not be confirmed. Do not repeat it; query the space first."
            : "The worker returned no query response.")
      }
      return try JSONDecoder().decode(JSONValue.self, from: Data(response.utf8))
    }
    throw HarnessError.message(
      "This saved tool is retired. No operation was executed. Query the space and request a new evaluation explicitly.")
  }
}
