import Foundation
import HarnessCore

@MainActor
extension RuntimeModel {
  func prepareSpaceCreation(_ proposal: SpaceProposal) async throws -> SpaceProposal {
    await refreshSpaces()
    guard catalogLoaded, let branch = catalogBranch, catalogError == nil, !loading, !signInPending
    else {
      throw HarnessError.message(
        "Sign in to Tonk and wait for your spaces to load, then try again.")
    }
    let identity = try await accountScript(
      """
      const account = await api('/api/account');
      const root = await api('/api/identity/root');
      if (account.status !== 'registered' || !root.rootDid)
        throw new Error('Sign in to Tonk before creating a space.');
      return {account: root.rootDid};
      """)
    guard let account = identity["account"] as? String else {
      throw HarnessError.message("Could not identify the active Tonk account.")
    }
    var prepared = proposal
    prepared.branch = branch
    prepared.account = account
    return prepared
  }

  func createSpace(_ proposal: SpaceProposal, submit: Bool) async throws -> TonkSpace {
    guard let branch = proposal.branch, let account = proposal.account else {
      throw HarnessError.message("This proposal has no creation receipt context.")
    }
    let arguments: [String: Any] = [
      "branch": branch, "account": account, "requestID": proposal.id, "name": proposal.name,
    ]
    let verifyAccount = """
      const root = await api('/api/identity/root');
      if (root.rootDid !== account) throw new Error('The active Tonk account changed.');
      """
    if submit {
      // Persisted before this call. A navigation or transport failure never causes a second submission.
      _ = try? await accountScript(
        verifyAccount + """
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
    }
    let deadline = Date().addingTimeInterval(60)
    while Date() < deadline {
      try Task.checkCancellation()
      // Navigation can briefly invalidate the page script; receipts live in the worker overlay.
      let result = try? await accountScript(
        verifyAccount + """
          const rows = await api('/api/profile/branch/' + encodeURIComponent(branch) + '/query', {
            predicate: {with: {
              status: {the: 'xyz.tonk.space-creation/status', as: 'Text', cardinality: 'one'},
              detail: {the: 'xyz.tonk.space-creation/detail', as: 'Text', cardinality: 'one'}
            }}, terms: {this: requestID, status: {'?': {name: 'status'}}, detail: {'?': {name: 'detail'}}}
          });
          return rows[0]?.fields || {};
          """, arguments: arguments)
      if let status = result?["status"] as? String, let detail = result?["detail"] as? String,
        let space = try proposal.createdSpace(status: status, detail: detail)
      {
        await refreshSpaces()
        guard spaces.contains(where: { $0.id == space.id }) else {
          try await Task.sleep(for: .seconds(1))
          continue
        }
        return space
      }
      try await Task.sleep(for: .seconds(1))
    }
    throw HarnessError.message(
      "Creation was submitted, but its result is not confirmed. Check status to look for the same request; no second space will be created."
    )
  }
}
