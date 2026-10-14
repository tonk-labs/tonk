import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  func requireSpaceReady(_ space: TonkSpace) throws {
    // A background catalog read does not invalidate confirmed live membership.
    // The worker still enforces authority for every request.
    guard catalogLoaded, !showingCachedCatalog, catalogError == nil,
      spaces.contains(where: { $0.id == space.id }),
      catalogBranch != nil, !loading, !signInPending
    else {
      throw HarnessError.message(
        "The attached space is not ready or is no longer in this account. Refresh spaces and try again."
      )
    }
  }

  func performSpaceTool(space: TonkSpace, name: String?) async throws -> String {
    let document: String
    if let name {
      // JSON strings are valid YAML scalars; no interpolation can add notation.
      let quoted = String(decoding: try JSONEncoder().encode(name), as: UTF8.self)
      document = "tonk/rename-repository!:\n  subject: \(space.subject)\n  name: \(quoted)\n"
      _ = try await performBuildTool(space, tool: "tonk_evaluate",
        arguments: .object(["document": .string(document)]))
    }
    let result = try await performBuildTool(space, tool: "tonk_query",
      arguments: .object(["document": .string("tonk/repository:\n  this: \(space.subject)\n")]))
    if name != nil { await refreshSpaces() }
    return String(decoding: try JSONEncoder().encode(result), as: UTF8.self)
  }

  func readSpaceSchema(_ space: TonkSpace) async throws -> String {
    let result = try await performBuildTool(space, tool: "tonk_query",
      arguments: .object(["document": .string("concept:")]))
    return String(decoding: try JSONEncoder().encode(result), as: UTF8.self)
  }
}
