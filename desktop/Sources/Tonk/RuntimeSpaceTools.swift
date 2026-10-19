import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  func requireSpaceReady(_ space: TonkSpace) throws {
    // A background catalog read does not invalidate confirmed live membership.
    // The worker still enforces authority for every request.
    guard catalogLoaded, !showingCachedCatalog, catalogError == nil,
      spaces.contains(where: { $0.id == space.id }),
      catalogBranch != nil, !loading, !spaceLoading, !signInPending
    else {
      throw HarnessError.message(
        "The attached space is not ready or is no longer in this account. Refresh spaces and try again."
      )
    }
  }

  func readSpaceInfo(_ space: TonkSpace) async throws -> String {
    let result = try await performBuildTool(
      space, tool: "tonk_query",
      arguments: .object(["document": .string("tonk/repository:\n  this: \(space.subject)\n")]))
    return String(decoding: try JSONEncoder().encode(result), as: UTF8.self)
  }

  func readSpaceSchema(_ space: TonkSpace) async throws -> String {
    let result = try await performBuildTool(
      space, tool: "tonk_query",
      arguments: .object(["document": .string("concept:")]))
    return String(decoding: try JSONEncoder().encode(result), as: UTF8.self)
  }
}
