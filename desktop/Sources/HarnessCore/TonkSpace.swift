import Foundation

public struct TonkSpace: Identifiable, Codable, Equatable, Sendable {
  public let subject: String
  public let name: String?
  public var id: String { subject }
  public var title: String {
    let title = name?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    return title.isEmpty ? "Untitled" : title
  }
  public var url: URL {
    RuntimeLocation.home.appendingPathComponent("space").appendingPathComponent(subject)
  }

  public static func decodeCatalog(_ data: Data) throws -> [Self] {
    let spaces = try JSONDecoder().decode([Self].self, from: data)
    var seen = Set<String>()
    for space in spaces {
      let key = space.subject.dropFirst("did:key:".count)
      guard space.subject.hasPrefix("did:key:z"), !key.isEmpty,
        key.allSatisfy({ "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz".contains($0) }
        ),
        seen.insert(space.id).inserted
      else { throw CallbackError("The space catalog contains an invalid or duplicate identity.") }
    }
    return spaces.sorted {
      let order = $0.title.localizedStandardCompare($1.title)
      return order == .orderedSame ? $0.id < $1.id : order == .orderedAscending
    }
  }
}
