import Combine
import Foundation

enum ShareRecipient: Hashable {
  case person
  case agent
}

/// Each recipient retains its own bearer link only for the lifetime of the sheet.
struct ShareInvitation {
  var link: String?
  var failed = false
  var copied = false

}

@MainActor
final class ShareInvitations: ObservableObject {
  @Published private(set) var creating = false
  @Published private(set) var showingProgress = false
  @Published private var invitations: [ShareRecipient: ShareInvitation] = [:]

  subscript(recipient: ShareRecipient) -> ShareInvitation {
    invitations[recipient] ?? ShareInvitation()
  }

  /// The first copy creates the link; later copies reuse it without another grant.
  func copyLink(
    for recipient: ShareRecipient,
    personLink: () async throws -> String,
    agentLink: () async throws -> String,
    copy: (String) -> Bool
  ) async -> Bool {
    guard !creating, !self[recipient].failed else { return false }
    creating = true
    let progress = Task {
      do { try await Task.sleep(for: .milliseconds(250)) } catch { return }
      showingProgress = true
    }
    defer {
      progress.cancel()
      showingProgress = false
      creating = false
    }
    if self[recipient].link == nil {
      do {
        let link: String
        switch recipient {
        case .person: link = try await personLink()
        case .agent: link = try await agentLink()
        }
        invitations[recipient, default: ShareInvitation()].link = link
      } catch {
        // An uncertain mint must not be automatically retried.
        invitations[recipient, default: ShareInvitation()].failed = true
        return false
      }
    }
    guard let link = self[recipient].link, copy(link) else { return false }
    invitations[recipient, default: ShareInvitation()].copied = true
    return true
  }
}
