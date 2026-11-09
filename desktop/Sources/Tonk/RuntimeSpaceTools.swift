import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  /// `page`, when given, is the window the request comes from; tools wait for it to load.
  func requireSpaceReady(_ space: TonkSpace, page: SpacePage? = nil) throws {
    // A background catalog read does not invalidate confirmed live membership.
    // The worker still enforces authority for every request.
    guard catalogLoaded, !showingCachedCatalog, catalogError == nil,
      spaces.contains(where: { $0.id == space.id }),
      catalogBranch != nil, !loading, page?.loading != true, !signInPending
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
