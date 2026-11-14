import Foundation

public enum ModelProvider: String, Codable, CaseIterable, Identifiable, Sendable {
  case disabled, chatGPT, claude, openAI, anthropic, gemini, openRouter, groq, mistral, ollama,
    grok, local,
    compatible
  public var id: String { rawValue }
  public var title: String {
    switch self {
    case .disabled: "Disabled"
    case .gemini: "Google Gemini"
    case .groq: "Groq"
    case .mistral: "Mistral"
    case .ollama: "Ollama"
    case .chatGPT: "ChatGPT Subscription"
    case .claude: "Claude Subscription"
    case .openAI: "OpenAI API"
    case .anthropic: "Anthropic"
    case .openRouter: "OpenRouter"
    case .grok: "Grok (xAI)"
    case .local: "Local server"
    case .compatible: "Custom OpenAI-compatible"
    }
  }
  public var baseURL: String {
    switch self {
    case .disabled, .chatGPT, .claude: ""
    case .gemini: "https://generativelanguage.googleapis.com/v1beta/openai"
    case .groq: "https://api.groq.com/openai/v1"
    case .mistral: "https://api.mistral.ai/v1"
    case .ollama: "http://localhost:11434/v1"
    case .openAI: "https://api.openai.com/v1"
    case .anthropic: "https://api.anthropic.com/v1"
    case .openRouter: "https://openrouter.ai/api/v1"
    case .grok: "https://api.x.ai/v1"
    case .local: "http://localhost:1234/v1"
    case .compatible: ""
    }
  }
  public var requiresKey: Bool {
    ![.disabled, .chatGPT, .claude, .ollama, .local, .compatible].contains(self)
  }
}

public struct ModelConnection: Codable, Equatable, Sendable {
  public var provider: ModelProvider
  public var baseURL: String
  public var model: String
  public init(
    provider: ModelProvider, baseURL: String? = nil, model: String = ""
  ) {
    self.provider = provider
    self.baseURL = baseURL ?? provider.baseURL
    self.model = model
  }
  public func endpoint() throws -> URL {
    guard ![.disabled, .chatGPT, .claude].contains(provider),
      !model.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    else {
      throw HarnessError.message("Enter a model ID from your provider or local server.")
    }
    guard let url = URL(string: baseURL), let host = url.host?.lowercased(),
      url.user == nil, url.password == nil, url.query == nil, url.fragment == nil
    else {
      throw HarnessError.message(
        "Enter a base URL without credentials, query parameters, or a fragment.")
    }
    let loopback = ["localhost", "127.0.0.1", "::1", "[::1]"].contains(host)
    guard url.scheme == "https" || (url.scheme == "http" && loopback),
      ![.local, .ollama].contains(provider) || loopback
    else {
      throw HarnessError.message(
        "Local servers must use a loopback address. Remote APIs require HTTPS.")
    }
    if ![.local, .ollama, .compatible].contains(provider), baseURL != provider.baseURL {
      throw HarnessError.message(
        "Use this provider's standard endpoint, or choose Other OpenAI-compatible API.")
    }
    return url.appendingPathComponent(provider == .anthropic ? "messages" : "chat/completions")
  }
}

/// The provider and model the account chose, as it syncs through the account
/// branch. Keys, base URLs and executable paths never leave the Mac.
public struct ModelChoice: Codable, Equatable, Sendable {
  public var provider: ModelProvider
  /// Empty means the provider's default.
  public var model: String
  public init(provider: ModelProvider, model: String = "") {
    self.provider = provider
    self.model = model
  }

  public enum Step: Equatable, Sendable {
    /// Nothing to do; `synced` should become this value.
    case agree(ModelChoice)
    /// Write the local choice to the account.
    case push(ModelChoice)
    /// Another desktop changed the account's choice; use it here.
    case adopt(ModelChoice)
    /// The account's choice is one this desktop cannot read; leave both.
    case ignore
  }

  /// What to do given this Mac's choice, the account's (`nil` when it has
  /// none), and the last value both agreed on.
  /// The first desktop to sync seeds the account; after that, a change on
  /// either side wins over the side that did not change.
  public static func reconcile(
    local: ModelChoice, remote: RemoteValue?, synced: ModelChoice?
  ) -> Step {
    guard let remote else { return .push(local) }
    guard case .known(let remote) = remote else { return .ignore }
    if remote == local { return .agree(remote) }
    if remote != synced { return .adopt(remote) }
    return .push(local)
  }

  public enum RemoteValue: Equatable, Sendable {
    case known(ModelChoice)
    case unknown

    public init(provider: String, model: String?) {
      if let provider = ModelProvider(rawValue: provider) {
        self = .known(ModelChoice(provider: provider, model: model ?? ""))
      } else {
        self = .unknown
      }
    }
  }
}

public struct APIToolCall: Codable, Equatable, Sendable {
  public var id: String
  public var name: String
  public var arguments: JSONValue
  public var extraContent: JSONValue?
  public init(id: String, name: String, arguments: JSONValue, extraContent: JSONValue? = nil) {
    self.id = id
    self.name = name
    self.arguments = arguments
    self.extraContent = extraContent
  }
}

public struct APIMessage: Codable, Equatable, Sendable {
  public var role: String
  public var text: String
  public var calls: [APIToolCall]
  public var toolID: String?
  public var isError: Bool
  public var extraContent: JSONValue?
  public init(
    role: String, text: String = "", calls: [APIToolCall] = [], toolID: String? = nil,
    isError: Bool = false, extraContent: JSONValue? = nil
  ) {
    self.role = role
    self.text = text
    self.calls = calls
    self.toolID = toolID
    self.isError = isError
    self.extraContent = extraContent
  }
  /// Never replay a tool whose result was lost during cancellation or shutdown.
  public static func repairingInterruptedTools(_ history: [APIMessage]) -> [APIMessage] {
    var result: [APIMessage] = []
    var pending: [APIToolCall] = []
    func unresolved() -> [APIMessage] {
      pending.map {
        APIMessage(
          role: "tool",
          text:
            "The previous turn was interrupted. Execution may have completed, but its result is unavailable. Do not replay a write; inspect current state first.",
          toolID: $0.id, isError: true)
      }
    }
    for message in history {
      if message.role == "tool" {
        pending.removeAll { $0.id == message.toolID }
      } else {
        result += unresolved()
        pending = message.calls
      }
      result.append(message)
    }
    result += unresolved()
    return result
  }
}
