import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  /// An open invitation is a ticket link, `/space/{subject}#{seed}`, or, when
  /// the space kept no ticket, a `/join` link carrying the grant. A link for one
  /// account (`recipient`) is always a `/join` link carrying a grant whose
  /// audience the worker echoes back. Either lives on this deployment or on
  /// the host serving the space.
  static func validateShareLink(
    _ response: [String: Any], subject: String, origins: [URL] = [RuntimeLocation.home],
    recipient: String? = nil
  ) throws -> URL {
    let kindMatches =
      if let recipient {
        response["kind"] as? String == "scoped"
          && response["recipientRoot"] as? String == recipient
      } else {
        response["kind"] as? String == "open"
      }
    guard kindMatches,
      let value = response["url"] as? String, let url = URL(string: value),
      url.user == nil, url.password == nil,
      origins.contains(where: {
        url.scheme == $0.scheme && url.host == $0.host && url.port == $0.port
      })
    else { throw HarnessError.message("Tonk returned an unsupported invitation link.") }
    let fragment = url.fragment ?? ""
    let carriesGrant =
      URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems?
      .contains(where: { $0.name == "access" && !($0.value ?? "").isEmpty }) ?? false
    let valid =
      switch url.path {
      case "/space/" + subject: recipient == nil && !fragment.isEmpty
      case "/join": recipient == nil ? !fragment.isEmpty || carriesGrant : carriesGrant
      default: false
      }
    guard valid else { throw HarnessError.message("Tonk returned an unsupported invitation link.") }
    return url
  }
}
