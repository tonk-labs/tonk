import Foundation
import HarnessCore

/// Create, rename and remove spaces from the sidebar. Each one submits the
/// worker command the web Hub uses, on the account branch, then waits until
/// the outcome shows in the catalog. None of them resubmits on a lost
/// response, so a slow worker can never create or remove twice.
@MainActor
extension RuntimeModel {
  func createSpace(named text: String) async throws -> TonkSpace {
    guard let name = SpaceName.validated(text) else { throw SpaceNameError() }
    let branch = try manageableBranch()
    let request = "urn:uuid:" + UUID().uuidString
    let arguments: [String: Any] = ["branch": branch, "requestID": request, "name": name]
    let (status, detail) = try await Self.submitAndConfirmSpaceCreation {
      _ = try await accountScript(
        """
        await api('/api/profile/branch/' + encodeURIComponent(branch) + '/transact', {
          claims: [{op: 'assert', application: {
            predicate: {kind: 'transient', concept: {
              description: 'A request to create a new space.',
              with: {name: {the: 'xyz.tonk.command.create-space/name', as: 'Text'}}
            }}, parameters: {this: requestID, name}
          }}]
        });
        return {submitted: true};
        """, arguments: arguments)
    } confirm: {
      try await receipt(
        request, on: branch, namespace: "xyz.tonk.space-creation",
        unconfirmed:
          "The space was requested, but Tonk hasn’t confirmed it yet. Check the sidebar before trying again."
      )
    }
    if status == "failed" { throw HarnessError.message(detail) }
    guard detail.hasPrefix("/space/") else {
      throw HarnessError.message("The worker returned an invalid creation receipt.")
    }
    let subject = String(detail.dropFirst("/space/".count))
    return try await awaitCatalog(
      "The space was created, but it hasn’t appeared in the sidebar yet."
    ) {
      $0.first { $0.subject == subject }
    }
  }

  /// A failed response does not establish that submission failed. Confirm the same
  /// request before reporting an outcome; never submit a second creation here.
  static func submitAndConfirmSpaceCreation(
    submit: () async throws -> Void,
    confirm: () async throws -> (String, String)
  ) async throws -> (String, String) {
    try Task.checkCancellation()
    _ = try? await submit()
    try Task.checkCancellation()
    return try await confirm()
  }

  /// Renames the space itself and its label on the account directory.
  func renameSpace(_ space: TonkSpace, to text: String) async throws {
    guard let name = SpaceName.validated(text) else { throw SpaceNameError() }
    guard name != space.title else { return }
    let branch = try manageableBranch()
    _ = try await accountScript(
      """
      await api('/api/profile/branch/' + encodeURIComponent(branch) + '/transact', {
        claims: [{op: 'assert', application: {
          predicate: {kind: 'transient', concept: {
            description: 'Rename a space from the profile Hub.',
            with: {
              name: {the: 'xyz.tonk.command.rename-repository/name', as: 'Text'},
              space: {the: 'xyz.tonk.rename-repository/space', as: 'Entity'}
            }
          }}, parameters: {this: 'urn:uuid:' + crypto.randomUUID(), name, space: subject}
        }}]
      });
      return {submitted: true};
      """, arguments: ["branch": branch, "subject": space.subject, "name": name])
    // The rename command writes no receipt; the directory label is its outcome.
    _ = try await awaitCatalog("The rename was sent, but the new name hasn’t appeared yet.") {
      $0.first { $0.subject == space.subject && $0.name == name }
    }
  }

  /// Deletes or leaves `space`, as `TonkSpace.removal` describes.
  func removeSpace(_ space: TonkSpace) async throws {
    let branch = try manageableBranch()
    let request = "urn:uuid:" + UUID().uuidString
    _ = try await accountScript(
      """
      await api('/api/profile/branch/' + encodeURIComponent(branch) + '/transact', {
        claims: [{op: 'assert', application: {
          predicate: {kind: 'transient', concept: {with: {
            subject: {the: 'xyz.tonk.command.remove-space/subject', as: 'Entity'}
          }}}, parameters: {this: requestID, subject}
        }}]
      });
      return {submitted: true};
      """, arguments: ["branch": branch, "requestID": request, "subject": space.subject])
    let (status, detail) = try await receipt(
      request, on: branch, namespace: "xyz.tonk.space-removal",
      unconfirmed: "Couldn’t confirm the removal. Check your spaces before trying again.")
    if status == "failed" { throw HarnessError.message(detail) }
    _ = try await awaitCatalog("The space was removed, but the sidebar hasn’t caught up yet.") {
      $0.contains { $0.subject == space.subject } ? nil : true
    }
  }

  private func manageableBranch() throws -> String {
    guard accountConnected, catalogLoaded, !showingCachedCatalog, let branch = catalogBranch else {
      throw HarnessError.message(
        "Sign in to Tonk and wait for your spaces to load, then try again.")
    }
    return branch
  }

  /// Polls a command's `<namespace>/status` and `/detail` receipt until it settles.
  private func receipt(
    _ request: String, on branch: String, namespace: String, unconfirmed: String
  ) async throws -> (String, String) {
    // A hosted create can take over a minute on a busy worker.
    let deadline = Date().addingTimeInterval(120)
    var lastError: Error?
    while Date() < deadline {
      try Task.checkCancellation()
      // Navigation can briefly invalidate the page script; receipts live in the worker overlay.
      do {
        let result = try await accountScript(
          """
          const rows = await api('/api/profile/branch/' + encodeURIComponent(branch) + '/query', {
            predicate: {with: {
              status: {the: namespace + '/status', as: 'Text', cardinality: 'one'},
              detail: {the: namespace + '/detail', as: 'Text', cardinality: 'one'}
            }}, terms: {this: requestID, status: {'?': {name: 'status'}}, detail: {'?': {name: 'detail'}}}
          });
          return rows[0]?.fields || {};
          """, arguments: ["branch": branch, "requestID": request, "namespace": namespace])
        if let status = result["status"] as? String, let detail = result["detail"] as? String {
          return (status, detail)
        }
      } catch { lastError = error }
      try await Task.sleep(for: .seconds(1))
    }
    let reason = lastError.map { " (" + AccessModel.message($0) + ")" } ?? ""
    throw HarnessError.message(unconfirmed + reason)
  }

  /// Refreshes the catalog until `match` finds the expected outcome in it.
  private func awaitCatalog<T>(
    _ unconfirmed: String, _ match: ([TonkSpace]) -> T?
  ) async throws -> T {
    for _ in 0..<30 {
      try Task.checkCancellation()
      await refreshSpaces()
      if let found = match(spaces) { return found }
      try await Task.sleep(for: .seconds(1))
    }
    throw HarnessError.message(unconfirmed)
  }
}

struct SpaceNameError: LocalizedError {
  var errorDescription: String? {
    "Enter a space name of up to \(SpaceName.maxLength) characters."
  }
}
