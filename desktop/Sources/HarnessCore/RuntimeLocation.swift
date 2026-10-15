import CryptoKit
import Foundation

public enum RuntimeLocation {
  public static let preferenceKey = "tonkRemote"

  public enum Deployment: Equatable, Sendable {
    case production, local
    case custom(URL)

    public var home: URL {
      switch self {
      case .production: URL(string: "https://tonk.network")!
      case .local: URL(string: "http://127.0.0.1:4187")!
      case .custom(let url): url
      }
    }
    public var host: String { home.host! }
    private var identifier: String {
      SHA256.hash(data: Data(home.absoluteString.utf8)).map { String(format: "%02x", $0) }.joined()
    }
    public var dataDirectory: String {
      switch self {
      case .production: "Tonk"
      case .local: "Tonk Local"
      case .custom: "Tonk Remote " + identifier
      }
    }
    public var webDataIdentifier: UUID? {
      switch self {
      case .production: return nil
      case .local: return UUID(uuidString: "65D52CA2-3801-4DD8-9400-473F2335BCAE")!
      case .custom:
        let hex = identifier
        let lengths = [8, 4, 4, 4, 12]
        var offset = hex.startIndex
        let parts = lengths.map { length in
          let end = hex.index(offset, offsetBy: length)
          defer { offset = end }
          return String(hex[offset..<end])
        }
        return UUID(uuidString: parts.joined(separator: "-"))!
      }
    }
    public var title: String {
      switch self {
      case .production: "Tonk"
      case .local: "Tonk — Local"
      case .custom: "Tonk — " + home.absoluteString
      }
    }
  }

  public static func remote(_ input: String) throws -> Deployment {
    let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
    let address = value.contains("://") ? value : "https://" + value
    guard var components = URLComponents(string: address),
      components.scheme?.lowercased() == "https",
      let host = components.host, !host.isEmpty,
      !host.contains(where: { $0.isWhitespace }),
      components.user == nil, components.password == nil,
      components.query == nil, components.fragment == nil,
      components.path.isEmpty || components.path == "/",
      components.port == nil || (1...65535).contains(components.port!)
    else {
      throw HarnessError.message(
        "Enter a remote host or HTTPS URL without a path, credentials, query, or fragment.")
    }
    components.scheme = "https"
    components.host = host.lowercased()
    components.path = ""
    if components.port == 443 { components.port = nil }
    guard let url = components.url else { throw HarnessError.message("Enter a valid remote URL.") }
    if Deployment.production.home == url { return .production }
    return .custom(url)
  }

  public static func resolve(arguments: [String], defaults: UserDefaults) -> Deployment {
    if arguments.contains("--local-runtime") { return .local }
    return (try? remote(defaults.string(forKey: preferenceKey) ?? "tonk.network")) ?? .production
  }

  // Freeze the selection until relaunch so runtime, CLI, and persisted chats use one remote.
  public static let deployment = resolve(
    arguments: ProcessInfo.processInfo.arguments, defaults: .standard)
  public static var home: URL { deployment.home }
  public static func isEmbedded(_ url: URL, deployment: Deployment = deployment) -> Bool {
    url.scheme == deployment.home.scheme && url.host == deployment.host
      && (url.port ?? (url.scheme == "https" ? 443 : 80))
        == (deployment.home.port ?? (deployment.home.scheme == "https" ? 443 : 80))
      && url.user == nil && url.password == nil
  }
  public static func isExternal(_ url: URL) -> Bool {
    ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "")
  }
}
