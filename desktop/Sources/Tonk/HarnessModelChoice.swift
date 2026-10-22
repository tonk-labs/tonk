import Foundation
import HarnessCore

/// Where the account keeps its model choice.
@MainActor
protocol ModelChoiceAccount: AnyObject {
  /// The account's choice, or `nil` when it has none yet.
  func readModelChoice() async throws -> ModelChoice.RemoteValue?
  func writeModelChoice(_ choice: ModelChoice) async throws
}

@MainActor
extension HarnessModel {
  /// Brings this Mac's provider and model in line with the account's, so
  /// every desktop on the account chats with the same model. A choice this
  /// Mac cannot use, such as one needing an API key it lacks, is left for
  /// the next sync; the local choice is untouched.
  func syncModelChoice() async {
    guard let runtime, runtime.accountConnected else { return }
    await syncModelChoice(with: runtime)
  }

  func syncModelChoice(with account: any ModelChoiceAccount) async {
    guard storageAvailable else { return }
    if modelChoiceSyncing {
      modelChoiceSyncAgain = true
      return
    }
    modelChoiceSyncing = true
    defer { modelChoiceSyncing = false }
    repeat {
      modelChoiceSyncAgain = false
      await reconcileModelChoice(with: account)
    } while modelChoiceSyncAgain
  }

  private func reconcileModelChoice(with account: any ModelChoiceAccount) async {
    let local = ModelChoice(provider: provider, model: connection.model)
    do {
      let remote = try await account.readModelChoice()
      switch ModelChoice.reconcile(local: local, remote: remote, synced: saved.syncedModelChoice) {
      case .agree(let choice):
        markModelChoiceSynced(choice)
      case .push(let choice):
        try await account.writeModelChoice(choice)
        markModelChoiceSynced(choice)
      case .adopt(let choice):
        var next =
          saved.connections?[choice.provider.rawValue] ?? ModelConnection(provider: choice.provider)
        next.model = choice.model
        try await configureProvider(
          choice.provider, connection: next, key: "", codexExecutable: saved.codexExecutable,
          trigger: .automatic)
        markModelChoiceSynced(choice)
      case .ignore:
        break
      }
    } catch {
      // Offline, busy, or unusable here: the next sync tries again.
    }
  }

  private func markModelChoiceSynced(_ choice: ModelChoice) {
    guard saved.syncedModelChoice != choice else { return }
    saved.syncedModelChoice = choice
    persist()
  }
}

extension RuntimeModel: ModelChoiceAccount {
  func readModelChoice() async throws -> ModelChoice.RemoteValue? {
    guard let branch = catalogBranch, let root = liveAccountRoot, !root.isEmpty else {
      throw HarnessError.message("The account is not loaded yet.")
    }
    let result = try await accountScript(
      """
      const field = (the, as, optional) => ({the, as, cardinality: 'one', ...(optional ? {optional: true} : {})});
      const rows = await api('/api/profile/branch/' + encodeURIComponent(branch) + '/query', {
        predicate: {with: {
          provider: field('xyz.tonk.model-choice/provider', 'Text'),
          model: field('xyz.tonk.model-choice/model', 'Text', true)
        }},
        terms: {this: root, provider: {'?': {name: 'provider'}}, model: {'?': {name: 'model'}}}
      });
      if (!Array.isArray(rows)) throw new Error('Invalid model choice response.');
      const row = rows[0]?.fields;
      return row ? {found: true, provider: row.provider, model: row.model || ''} : {found: false};
      """, arguments: ["branch": branch, "root": root])
    guard result["found"] as? Bool == true else { return nil }
    guard let provider = result["provider"] as? String else { return .unknown }
    return ModelChoice.RemoteValue(provider: provider, model: result["model"] as? String)
  }

  func writeModelChoice(_ choice: ModelChoice) async throws {
    guard let branch = catalogBranch, let root = liveAccountRoot, !root.isEmpty else {
      throw HarnessError.message("The account is not loaded yet.")
    }
    _ = try await accountScript(
      """
      const base = '/api/profile/branch/' + encodeURIComponent(branch);
      const field = (the, as) => ({the, as, cardinality: 'one'});
      const text = (key, the) => ({kind: 'durable', concept: {with: {[key]: field(the, 'Text')}}});
      // Read the stored model now, so choosing the default retracts exactly what is there.
      // A query needs one required field; the model is only stored beside a provider.
      const rows = await api(base + '/query', {
        predicate: {with: {
          provider: field('xyz.tonk.model-choice/provider', 'Text'),
          model: {...field('xyz.tonk.model-choice/model', 'Text'), optional: true}
        }},
        terms: {this: root, provider: {'?': {name: 'provider'}}, model: {'?': {name: 'model'}}}
      });
      const stored = rows[0]?.fields?.model;
      const claims = [{op: 'assert', application: {
        predicate: text('provider', 'xyz.tonk.model-choice/provider'), parameters: {this: root, provider}}}];
      if (model) {
        claims.push({op: 'assert', application: {
          predicate: text('model', 'xyz.tonk.model-choice/model'), parameters: {this: root, model}}});
      } else if (stored) {
        claims.push({op: 'retract', application: {
          predicate: text('model', 'xyz.tonk.model-choice/model'), parameters: {this: root, model: stored}}});
      }
      await api(base + '/transact', {claims});
      return {saved: claims.length};
      """,
      arguments: [
        "branch": branch, "root": root, "provider": choice.provider.rawValue,
        "model": choice.model,
      ])
  }
}
