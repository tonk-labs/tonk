import Foundation

/// A directory assertion retained with a contact, not a grant or a permanent
/// guarantee that the email still belongs to this key.
public struct EmailContactIdentity: Codable, Equatable, Sendable {
  public let email: String
  public let rootDid: String
  public let directory: URL
  public let checkedAt: Date
}

public enum EmailContactResult: Equatable, Sendable {
  case found(EmailContactIdentity)
  case pending, notFound, suspended
}

public enum EmailContactError: LocalizedError {
  case invalidEmail, unavailable, invalidDocument

  public var errorDescription: String? {
    switch self {
    case .invalidEmail: "Enter an email address or a did:key."
    case .unavailable: "Email lookup is unavailable. Try again."
    case .invalidDocument: "The directory returned an inconsistent account. Try again later."
    }
  }
}

/// Uses the same deployment origin as the worker's registration lookup.
/// This is a public read: it sends no account credentials or cookies.
public struct EmailContactLookup: Sendable {
  public let directory: URL

  public init(directory: URL = RuntimeLocation.home) {
    self.directory = directory
  }

  public func lookup(_ input: String) async throws -> EmailContactResult {
    let query = try Query(input, directory: directory)
    let configuration = URLSessionConfiguration.ephemeral
    configuration.urlCache = nil
    configuration.httpCookieStorage = nil
    configuration.urlCredentialStorage = nil
    let session = URLSession(
      configuration: configuration, delegate: NoContactRedirects(), delegateQueue: nil)
    defer { session.invalidateAndCancel() }
    var request = URLRequest(
      url: query.url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 15)
    request.setValue("application/did+json, application/json", forHTTPHeaderField: "Accept")
    let data: Data
    let response: URLResponse
    do {
      (data, response) = try await session.data(for: request)
    } catch {
      if Task.isCancelled { throw CancellationError() }
      throw EmailContactError.unavailable
    }
    guard let http = response as? HTTPURLResponse, http.url == query.url else {
      throw EmailContactError.unavailable
    }
    return try Self.decode(data, status: http.statusCode, query: query, directory: directory)
  }

  struct Query {
    let email: String
    let did: String
    let url: URL

    init(_ input: String, directory: URL) throws {
      let email = input.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
      let parts = email.split(separator: "@", omittingEmptySubsequences: false)
      guard email.utf8.count <= 254, parts.count == 2,
        !parts[0].isEmpty, !parts[1].isEmpty,
        !email.contains(where: { $0.isWhitespace || $0.isNewline }),
        !email.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
        !parts[0].contains("/"), !parts[0].contains("\\"),
        parts[0] != ".", parts[0] != "..",
        parts[1].utf8.allSatisfy({
          (97...122).contains($0) || (48...57).contains($0) || $0 == 45 || $0 == 46
        }),
        parts[1].split(separator: ".", omittingEmptySubsequences: false).allSatisfy({ !$0.isEmpty })
      else { throw EmailContactError.invalidEmail }
      guard var components = URLComponents(url: directory, resolvingAgainstBaseURL: false),
        let host = components.host,
        components.scheme == "https"
          || (components.scheme == "http" && ["127.0.0.1", "localhost"].contains(host)),
        components.user == nil, components.password == nil,
        components.query == nil, components.fragment == nil,
        components.path.isEmpty || components.path == "/"
      else { throw EmailContactError.unavailable }
      let local = Self.encode(String(parts[0]))
      let domain = String(parts[1])
      components.percentEncodedPath = "/customer/\(domain)/\(local)/did.json"
      guard let url = components.url else { throw EmailContactError.invalidEmail }
      let authority = host + (components.port.map { ":\($0)" } ?? "")
      self.email = email
      self.did =
        "did:web:\(authority.replacingOccurrences(of: ":", with: "%3A")):customer:\(domain):\(local)"
      self.url = url
    }

    // Same byte-wise idchar encoding as tonk-access-service::lookup::encode_local.
    private static func encode(_ value: String) -> String {
      value.utf8.map { byte in
        if (97...122).contains(byte) || (48...57).contains(byte) || [45, 46, 95].contains(byte) {
          return String(UnicodeScalar(byte))
        }
        return String(format: "%%%02X", byte)
      }.joined()
    }
  }

  static func decode(_ data: Data, status: Int, query: Query, directory: URL) throws
    -> EmailContactResult
  {
    switch status {
    case 202: return .pending
    case 404: return .notFound
    case 410: return .suspended
    case 200: break
    default: throw EmailContactError.unavailable
    }
    guard let document = try? JSONDecoder().decode(Document.self, from: data),
      document.id == query.did, document.status == "Active", document.deactivated != true,
      document.alsoKnownAs.count == 1, let root = document.alsoKnownAs.first,
      root.hasPrefix("did:key:z"), root.count > 9,
      document.verificationMethod.contains(where: {
        $0.type == "Multikey" && $0.controller == query.did
          && "did:key:" + $0.publicKeyMultibase == root
          && $0.id == query.did + "#" + $0.publicKeyMultibase
      })
    else { throw EmailContactError.invalidDocument }
    return .found(
      EmailContactIdentity(
        email: query.email, rootDid: root, directory: directory, checkedAt: Date()))
  }

  private struct Document: Decodable {
    let id: String
    let status: String
    let deactivated: Bool?
    let alsoKnownAs: [String]
    let verificationMethod: [Method]
    struct Method: Decodable {
      let id: String
      let type: String
      let controller: String
      let publicKeyMultibase: String
    }
  }
}

private final class NoContactRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
  func urlSession(
    _ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse,
    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
  ) {
    completionHandler(nil)
  }
}
