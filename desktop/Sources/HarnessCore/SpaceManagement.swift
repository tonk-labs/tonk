import Foundation

/// A space name typed into the desktop's create or rename sheet.
public enum SpaceName {
  public static let maxLength = 80

  /// The trimmed name, or nil when it is empty, too long or has control characters.
  public static func validated(_ text: String) -> String? {
    let name = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !name.isEmpty, name.count <= maxLength,
      !name.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) })
    else { return nil }
    return name
  }
}

/// What removing a space does, decided the same way as the web Hub's
/// `<space-remove>`: one worker command serves all three, and the worker
/// releases the hosted copy only when this account provides the space.
public enum SpaceRemoval: Equatable, Sendable {
  /// This account hosts the space: its copy on Tonk services goes too.
  case deleteHosted
  /// Made here and never hosted: the only copy is on this Mac.
  case deleteLocal
  /// Joined through an invite: other members keep it.
  case leave

  public var isLeave: Bool { self == .leave }

  /// The context-menu and confirmation button label.
  public var verb: String { isLeave ? "Leave" : "Delete" }

  public func title(_ name: String) -> String {
    isLeave ? "Leave “\(name)”?" : "Delete “\(name)”?"
  }

  public var message: String {
    switch self {
    case .deleteHosted:
      "This account hosts this space, so its copy on Tonk services is deleted along with the data on this Mac. Other members lose access. This can’t be undone."
    case .deleteLocal:
      "This space has no copy on Tonk services, so its data on this Mac is gone for good. This can’t be undone."
    case .leave:
      "This removes the space and its data from this Mac. Other members keep access. You’ll need a new invite link to join again."
    }
  }
}

extension TonkSpace {
  public func removal(accountRoot: String?) -> SpaceRemoval {
    guard let provider, !provider.isEmpty else { return founded == true ? .deleteLocal : .leave }
    return provider == accountRoot ? .deleteHosted : .leave
  }
}
