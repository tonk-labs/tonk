import CryptoKit
import Foundation

/// A presentation snapshot, never evidence of a valid session or membership.
public struct SpaceCatalogCache {
  public struct Snapshot: Codable {
    public let version: Int
    public let accountRoot: String
    public let branch: String
    public let spaces: [TonkSpace]

    public init(accountRoot: String, branch: String, spaces: [TonkSpace]) {
      version = 1
      self.accountRoot = accountRoot
      self.branch = branch
      self.spaces = spaces
    }
  }

  private let file: URL

  public init(directory: URL, scope: String) {
    let key = SHA256.hash(data: Data(scope.utf8)).map { String(format: "%02x", $0) }.joined()
    file = directory.appendingPathComponent("SpaceCatalogs/\(key).json")
  }

  public func load() -> Snapshot? {
    guard let data = try? Data(contentsOf: file),
      let value = try? JSONDecoder().decode(Snapshot.self, from: data),
      value.version == 1, !value.accountRoot.isEmpty, !value.branch.isEmpty,
      let rows = try? JSONEncoder().encode(value.spaces),
      (try? TonkSpace.decodeCatalog(rows)) != nil
    else { return nil }
    return value
  }

  public func save(_ snapshot: Snapshot) throws {
    try FileManager.default.createDirectory(
      at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
    try JSONEncoder().encode(snapshot).write(to: file, options: .atomic)
  }

  public func clear() {
    try? FileManager.default.removeItem(at: file)
  }
}
