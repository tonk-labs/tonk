import Foundation
import HarnessCore

struct SubscriptionModel: Identifiable, Equatable {
  let id: String
  let name: String
  let isDefault: Bool
}

@MainActor
extension HarnessModel {
  var modelLabel: String {
    if provider == .disabled { return provider.title }
    if provider == .claude { return connection.model.isEmpty ? "Claude default" : connection.model }
    if provider != .chatGPT {
      return connection.model.isEmpty ? "Choose a model" : connection.model
    }
    return saved.conversation.resolvedModel
      ?? (connection.model.isEmpty ? "Model not selected" : connection.model)
  }

  var configuredModels: [ModelConnection] {
    var models = saved.modelPresets ?? []
    let remembered =
      (saved.connections ?? [:]).values.sorted { $0.provider.rawValue < $1.provider.rawValue }
      + (saved.sessions ?? []).compactMap(\.connection)
    for connection in remembered {
      if !connection.model.isEmpty && !models.contains(connection) { models.append(connection) }
    }
    return models
  }

  func refreshSubscriptionModels() async {
    guard provider == .chatGPT, connected, signedIn, !subscriptionModelsLoading else { return }
    subscriptionModelsLoading = true
    defer { subscriptionModelsLoading = false }
    do {
      var models: [SubscriptionModel] = []
      var cursor: String?
      for _ in 0..<10 {
        var params: [String: JSONValue] = ["limit": .number(100)]
        if let cursor { params["cursor"] = .string(cursor) }
        let result = try await client.request("model/list", params: .object(params))
        for row in result["data"].array {
          guard row["hidden"].bool != true, let id = row["model"].string,
            !models.contains(where: { $0.id == id })
          else { continue }
          models.append(
            SubscriptionModel(
              id: id, name: row["displayName"].string ?? id,
              isDefault: row["isDefault"].bool == true))
        }
        cursor = result["nextCursor"].string
        if cursor == nil { break }
      }
      subscriptionModels = models
      modelCatalogError = nil
      if provider == .chatGPT && connection.model.isEmpty {
        let config = try await client.request(
          "config/read", params: .object(["includeLayers": .bool(false)]))
        if let id = config["config"]["model"].string ?? models.first(where: \.isDefault)?.id {
          var connections = saved.connections ?? [:]
          connections[ModelProvider.chatGPT.rawValue] = ModelConnection(
            provider: .chatGPT, model: id)
          saved.connections = connections
          persist()
        }
      }
    } catch { modelCatalogError = "Could not load ChatGPT models. Reconnect to try again." }
  }
}
