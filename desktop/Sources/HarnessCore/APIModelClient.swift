import Foundation

public enum APIModelWire {
  public static func request(
    connection: ModelConnection, key: String, instructions: String,
    history: [APIMessage], tools: [JSONValue]
  ) throws -> URLRequest {
    var request = URLRequest(url: try connection.endpoint())
    if connection.provider.requiresKey && key.isEmpty {
      throw HarnessError.message("Add an API key for \(connection.provider.title).")
    }
    request.httpMethod = "POST"
    request.timeoutInterval = 120
    request.setValue("application/json", forHTTPHeaderField: "Content-Type")
    request.setValue("text/event-stream", forHTTPHeaderField: "Accept")
    let anthropic = connection.provider == .anthropic
    if anthropic {
      request.setValue(key, forHTTPHeaderField: "x-api-key")
      request.setValue("2023-06-01", forHTTPHeaderField: "anthropic-version")
    } else if !key.isEmpty {
      request.setValue("Bearer \(key)", forHTTPHeaderField: "Authorization")
    }
    var messages = history.map { message -> JSONValue in
      if anthropic {
        if message.role == "tool" {
          return .object([
            "role": .string("user"),
            "content": .array([
              .object([
                "type": .string("tool_result"), "tool_use_id": .string(message.toolID ?? ""),
                "content": .string(message.text), "is_error": .bool(message.isError),
              ])
            ]),
          ])
        }
        var content: [JSONValue] =
          message.text.isEmpty
          ? [] : [.object(["type": .string("text"), "text": .string(message.text)])]
        content += message.calls.map {
          .object([
            "type": .string("tool_use"), "id": .string($0.id), "name": .string($0.name),
            "input": $0.arguments,
          ])
        }
        return .object(["role": .string(message.role), "content": .array(content)])
      }
      var object: [String: JSONValue] = [
        "role": .string(message.role), "content": .string(message.text),
      ]
      if let extra = message.extraContent { object["extra_content"] = extra }
      if let id = message.toolID { object["tool_call_id"] = .string(id) }
      if !message.calls.isEmpty {
        object["tool_calls"] = .array(
          message.calls.map { call in
            var encoded: [String: JSONValue] = [
              "id": .string(call.id), "type": .string("function"),
              "function": .object([
                "name": .string(call.name), "arguments": .string(json(call.arguments)),
              ]),
            ]
            if let extra = call.extraContent { encoded["extra_content"] = extra }
            return .object(encoded)
          })
      }
      return .object(object)
    }
    if !anthropic {
      messages.insert(
        .object(["role": .string("system"), "content": .string(instructions)]), at: 0)
    }
    var body: [String: JSONValue] = [
      "model": .string(connection.model), "stream": .bool(true), "messages": .array(messages),
    ]
    if anthropic {
      body["system"] = .string(instructions)
      body["max_tokens"] = .number(8192)
    }
    if !tools.isEmpty {
      body["tools"] = .array(
        tools.map { tool in
          if anthropic {
            return .object([
              "name": tool["name"], "description": tool["description"],
              "input_schema": tool["inputSchema"],
            ])
          }
          return .object([
            "type": .string("function"),
            "function": .object([
              "name": tool["name"], "description": tool["description"],
              "parameters": tool["inputSchema"],
            ]),
          ])
        })
    }
    if connection.provider == .openRouter && !tools.isEmpty {
      body["provider"] = .object(["require_parameters": .bool(true)])
    }
    request.httpBody = try JSONEncoder().encode(JSONValue.object(body))
    guard request.httpBody!.count <= 16 * 1024 * 1024 else {
      throw HarnessError.message("This conversation is too large. Start a new conversation.")
    }
    return request
  }
  public static func json(_ value: JSONValue) -> String {
    String(decoding: (try? JSONEncoder().encode(value)) ?? Data(), as: UTF8.self)
  }
}

/// Decode only complete provider turns. Partial tool arguments are never executable.
public struct APIStreamDecoder {
  public let anthropic: Bool
  public private(set) var text = ""
  public private(set) var finished = false
  private var extraContent: JSONValue?
  private var callExtras: [Int: JSONValue] = [:]
  private var reason: String?
  private var calls: [Int: (id: String, name: String, arguments: String)] = [:]
  public init(anthropic: Bool) { self.anthropic = anthropic }

  public mutating func accept(_ data: String) throws -> String {
    if data == "[DONE]" {
      finished = true
      return ""
    }
    let value = try JSONDecoder().decode(JSONValue.self, from: Data(data.utf8))
    if value["error"] != .null || value["type"].string == "error" {
      throw HarnessError.message(
        "The model provider interrupted its response. No pending tool calls were run.")
    }
    var delta = ""
    if anthropic {
      switch value["type"].string {
      case "content_block_start":
        let block = value["content_block"]
        if block["type"].string == "text" { delta = block["text"].string ?? "" }
        if block["type"].string == "tool_use", let index = value["index"].int {
          calls[index] = (block["id"].string ?? "", block["name"].string ?? "", "")
        }
      case "content_block_delta":
        delta = value["delta"]["text"].string ?? ""
        if let index = value["index"].int, var call = calls[index] {
          call.arguments += value["delta"]["partial_json"].string ?? ""
          calls[index] = call
        }
      case "message_delta": reason = value["delta"]["stop_reason"].string ?? reason
      case "message_stop": finished = true
      default: break
      }
    } else {
      guard let choice = value["choices"].array.first else { return "" }
      if choice["delta"]["extra_content"] != .null {
        extraContent = choice["delta"]["extra_content"]
      }
      delta = choice["delta"]["content"].string ?? ""
      reason = choice["finish_reason"].string ?? reason
      for tool in choice["delta"]["tool_calls"].array {
        guard let index = tool["index"].int else {
          throw HarnessError.message("The provider returned an invalid tool call.")
        }
        if tool["extra_content"] != .null { callExtras[index] = tool["extra_content"] }
        var call = calls[index] ?? ("", "", "")
        call.id += tool["id"].string ?? ""
        call.name += tool["function"]["name"].string ?? ""
        call.arguments += tool["function"]["arguments"].string ?? ""
        calls[index] = call
      }
    }
    text += delta
    guard calls.count <= 32, text.utf8.count <= 4 * 1024 * 1024,
      calls.values.allSatisfy({ $0.arguments.utf8.count <= 1024 * 1024 })
    else {
      throw HarnessError.message("The model response exceeded the supported size.")
    }
    return delta
  }

  public func result() throws -> APIMessage {
    guard finished, let reason,
      ["stop", "tool_calls", "end_turn", "tool_use", "stop_sequence"].contains(reason)
    else {
      throw HarnessError.message(
        "The model response ended early or reached its output limit. Pending tools were not run.")
    }
    let parsed = try calls.sorted { $0.key < $1.key }.map { index, call -> APIToolCall in
      guard !call.id.isEmpty, !call.name.isEmpty else {
        throw HarnessError.message("The provider returned an incomplete tool call.")
      }
      let arguments = try JSONDecoder().decode(
        JSONValue.self, from: Data((call.arguments.isEmpty ? "{}" : call.arguments).utf8))
      guard case .object = arguments else {
        throw HarnessError.message("Tool arguments must be a JSON object.")
      }
      return APIToolCall(
        id: call.id, name: call.name, arguments: arguments, extraContent: callExtras[index])
    }
    guard Set(parsed.map(\.id)).count == parsed.count,
      parsed.isEmpty || ["tool_calls", "tool_use"].contains(reason)
    else {
      throw HarnessError.message("The provider returned inconsistent tool calls.")
    }
    guard !text.isEmpty || !parsed.isEmpty else {
      throw HarnessError.message("The model returned an empty reply.")
    }
    return APIMessage(role: "assistant", text: text, calls: parsed, extraContent: extraContent)
  }
}

private final class NoModelRedirects: NSObject, URLSessionTaskDelegate, @unchecked Sendable {
  func urlSession(
    _ session: URLSession, task: URLSessionTask,
    willPerformHTTPRedirection response: HTTPURLResponse,
    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void
  ) {
    completionHandler(nil)
  }
}

@MainActor
public final class APIModelClient {
  public typealias Completion = (URLRequest, Bool, @escaping (String) -> Void) async throws ->
    APIMessage
  private let completion: Completion
  public init(completion: Completion? = nil) { self.completion = completion ?? Self.fetch }

  public func run(
    connection: ModelConnection, key: String, instructions: String, history: [APIMessage],
    tools: [JSONValue], onHistory: ([APIMessage]) throws -> Void,
    onText: @escaping (String, String) -> Void,
    execute: (APIToolCall) async -> JSONValue
  ) async throws {
    var history = history
    let allowed = Set(tools.compactMap { $0["name"].string })
    for _ in 0..<24 {
      try Task.checkCancellation()
      let id = UUID().uuidString
      let request = try APIModelWire.request(
        connection: connection, key: key, instructions: instructions, history: history, tools: tools
      )
      let reply = try await completion(request, connection.provider == .anthropic) { delta in
        onText(id, delta)
      }
      try Task.checkCancellation()
      history.append(reply)
      try onHistory(history)
      if reply.calls.isEmpty { return }
      for call in reply.calls {
        try Task.checkCancellation()
        let result: JSONValue
        if allowed.contains(call.name) {
          result = await execute(call)
        } else {
          result = SpaceTools.response("This tool is unavailable.", success: false)
        }
        // Save returned outcomes even if cancellation raced a completed mutation.
        history.append(
          APIMessage(
            role: "tool", text: APIModelWire.json(result), toolID: call.id,
            isError: result["success"].bool != true))
        try onHistory(history)
      }
    }
    throw HarnessError.message(
      "The agent reached the tool-step limit. Review its progress before continuing.")
  }

  static func fetch(_ request: URLRequest, anthropic: Bool, onDelta: @escaping (String) -> Void)
    async throws -> APIMessage
  {
    let configuration = URLSessionConfiguration.ephemeral
    configuration.httpShouldSetCookies = false
    configuration.timeoutIntervalForResource = 300
    let session = URLSession(
      configuration: configuration, delegate: NoModelRedirects(), delegateQueue: nil)
    defer { session.invalidateAndCancel() }
    let (bytes, response) = try await session.bytes(for: request)
    guard let http = response as? HTTPURLResponse, (200..<300).contains(http.statusCode) else {
      let status = (response as? HTTPURLResponse)?.statusCode ?? 0
      throw HarnessError.message(
        "Model request failed (HTTP \(status)). Check the endpoint, key, and model ID. Tonk requires a model and endpoint that support tool calling. The request was not retried."
      )
    }
    var decoder = APIStreamDecoder(anthropic: anthropic)
    var event: [String] = []
    var size = 0
    // AsyncBytes.lines skips empty lines; SSE requires them as event boundaries.
    var lineBytes = Data()
    func flushEvent() throws {
      guard !event.isEmpty else { return }
      let delta = try decoder.accept(event.joined(separator: "\n"))
      event.removeAll()
      if !delta.isEmpty { onDelta(delta) }
    }
    func consumeLine() throws {
      if lineBytes.last == 13 { lineBytes.removeLast() }
      let line = String(decoding: lineBytes, as: UTF8.self)
      lineBytes.removeAll(keepingCapacity: true)
      if line.isEmpty {
        try flushEvent()
      } else if line.hasPrefix("data:") {
        var value = String(line.dropFirst(5))
        if value.hasPrefix(" ") { value.removeFirst() }
        event.append(value)
      }
    }
    for try await byte in bytes {
      try Task.checkCancellation()
      size += 1
      guard size <= 16 * 1024 * 1024 else {
        throw HarnessError.message("The model response exceeded the supported size.")
      }
      if byte == 10 { try consumeLine() } else { lineBytes.append(byte) }
      if decoder.finished { break }
    }
    if !lineBytes.isEmpty { try consumeLine() }
    try flushEvent()
    return try decoder.result()
  }
}
