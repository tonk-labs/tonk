import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  var showsSpacePicker: Bool {
    accountConnected || showingCachedCatalog || RuntimeLocation.deployment == .local
  }

  func prepareCatalogCache(directory: URL) {
    guard catalogCache == nil else { return }
    let arguments = ProcessInfo.processInfo.arguments
    let profile: String
    if let index = arguments.firstIndex(of: "--web-data-id"),
      arguments.indices.contains(index + 1), let id = UUID(uuidString: arguments[index + 1])
    {
      profile = id.uuidString
    } else {
      profile = RuntimeLocation.deployment.webDataIdentifier?.uuidString ?? "default"
    }
    let cache = SpaceCatalogCache(
      directory: directory, scope: RuntimeLocation.home.absoluteString + "|" + profile)
    catalogCache = cache
    if let snapshot = cache.load(), !snapshot.spaces.isEmpty {
      cachedAccountRoot = snapshot.accountRoot
      spaces = snapshot.spaces
      catalogBranch = snapshot.branch
      catalogLoaded = true
      showingCachedCatalog = true
    }
  }

  func acceptCatalog(_ rows: [TonkSpace], branch: String, root: String, status: String) throws {
    if status == "registered" && root.isEmpty { throw CallbackError("Missing account identity.") }
    try applyAccountStatus(["status": status])
    liveAccountRoot = accountConnected ? root : nil
    spaces = rows
    catalogBranch = branch
    catalogLoaded = true
    showingCachedCatalog = false
    if accountConnected {
      // Cache failures must not turn a successful live query into a failed login.
      try? catalogCache?.save(.init(accountRoot: root, branch: branch, spaces: rows))
    } else {
      catalogCache?.clear()
      cachedAccountRoot = nil
    }
  }

  /// Early clicks wait for the live catalog, never granting access from cached data.
  func spaceForOpening(_ requested: TonkSpace) async -> TonkSpace? {
    guard pendingSpace == nil else { return nil }
    let expectedRoot = showingCachedCatalog ? cachedAccountRoot : nil
    pendingSpace = requested
    defer { pendingSpace = nil }
    for _ in 0..<3000 {
      if Task.isCancelled { return nil }
      if !showingCachedCatalog && catalogLoaded {
        guard showsSpacePicker,
          expectedRoot == nil || expectedRoot == liveAccountRoot,
          let current = spaces.first(where: { $0.id == requested.id })
        else {
          catalogError = "This space is no longer available in the current account."
          return nil
        }
        return current
      }
      if catalogError != nil || error != nil || accountMessage != nil { return nil }
      do { try await Task.sleep(for: .milliseconds(20)) } catch { return nil }
    }
    catalogError = "Tonk is still starting. Try opening the space again."
    return nil
  }
}
