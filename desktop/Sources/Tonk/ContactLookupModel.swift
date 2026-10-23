import Combine
import Foundation
import HarnessCore

@MainActor
final class ContactLookupModel: ObservableObject {
  @Published var input = "" { didSet { invalidate() } }
  @Published private(set) var result: EmailContactResult?
  @Published private(set) var error: String?
  @Published private(set) var isLoading = false
  private var revision = 0
  private let resolve: (String) async throws -> EmailContactResult

  init(
    resolve: @escaping (String) async throws -> EmailContactResult = {
      try await EmailContactLookup().lookup($0)
    }
  ) {
    self.resolve = resolve
  }

  var manualDid: String? {
    let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
    return value.hasPrefix("did:key:") && value.count > 8
      && !value.contains(where: { $0.isWhitespace }) ? value : nil
  }
  /// A complete-looking address, so typing can look it up without a button.
  var isCompleteEmail: Bool {
    let value = input.trimmingCharacters(in: .whitespacesAndNewlines)
    let parts = value.split(separator: "@", omittingEmptySubsequences: false)
    guard parts.count == 2, !parts[0].isEmpty, !value.contains(where: { $0.isWhitespace })
    else { return false }
    let labels = parts[1].split(separator: ".", omittingEmptySubsequences: false)
    return labels.count >= 2 && labels.allSatisfy { !$0.isEmpty } && labels.last!.count >= 2
  }
  var identity: EmailContactIdentity? {
    if case .found(let identity) = result { return identity }
    return nil
  }
  var did: String? { manualDid ?? identity?.rootDid }

  func invalidate() {
    revision += 1
    result = nil
    error = nil
    isLoading = false
  }

  func lookup() async {
    guard !Task.isCancelled, !isLoading, manualDid == nil else { return }
    invalidate()
    let started = revision
    let address = input
    isLoading = true
    defer { if revision == started { isLoading = false } }
    do {
      let answer = try await resolve(address)
      guard revision == started, !Task.isCancelled else { return }
      result = answer
    } catch {
      guard revision == started, !Task.isCancelled else { return }
      self.error = error.localizedDescription
    }
  }
}
