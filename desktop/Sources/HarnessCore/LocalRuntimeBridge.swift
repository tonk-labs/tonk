// Desktop-owned private transport; canonical tools are generated from mcp/.
import Foundation
import Network

/// Private loopback transport for a single host-provided space capability.
/// The external stdio adapter and native HTTP adapter share the monorepo contract.
@MainActor
public final class LocalRuntimeBridge {
  public typealias Handler = @MainActor (String, JSONValue) async throws -> JSONValue
  private let agentTools: [JSONValue]?
  private let handler: Handler
  private let allowsWrites: Bool
  private let supportsInspection: Bool
  private let token = UUID().uuidString + UUID().uuidString
  private var listener: NWListener?
  private var startup: CheckedContinuation<JSONValue, Error>?
  private var authority = ""
  private var connections: [UUID: NWConnection] = [:]
  private var jobs: [UUID: Task<Void, Never>] = [:]

  public init(
    allowsWrites: Bool = false, supportsInspection: Bool = false, agentTools: [JSONValue]? = nil,
    handler: @escaping Handler
  ) {
    self.allowsWrites = allowsWrites
    self.supportsInspection = supportsInspection
    self.agentTools = agentTools?.filter { tool in
      let name = tool["name"].string ?? ""
      return (["tonk_query", "tonk_guide", "tonk_inspect_view", "tonk_propose_space"].contains(name)
        || (allowsWrites && name == "tonk_evaluate"))
    }
    self.handler = handler
  }

  public func start() async throws -> JSONValue {
    guard listener == nil else { throw HarnessError.message("Runtime bridge is already running.") }
    let parameters = NWParameters.tcp
    parameters.requiredLocalEndpoint = .hostPort(host: "127.0.0.1", port: .any)
    let server = try NWListener(using: parameters)
    listener = server
    server.newConnectionHandler = { [weak self] connection in
      Task { @MainActor in self?.accept(connection) }
    }
    server.stateUpdateHandler = { [weak self, weak server] state in
      Task { @MainActor in
        guard let self else { return }
        switch state {
        case .ready:
          guard let port = server?.port, let startup = self.startup else { return }
          self.authority = "127.0.0.1:\(port.rawValue)"
          self.startup = nil
          startup.resume(
            returning: .object([
              "version": .number(1), "url": .string("http://" + self.authority),
              "token": .string(self.token),
            ]))
        case .failed(let error): self.stop(error: error)
        default: break
        }
      }
    }
    return try await withCheckedThrowingContinuation {
      startup = $0
      server.start(queue: .main)
    }
  }

  public func stop() { stop(error: CancellationError()) }

  private func stop(error: Error) {
    startup?.resume(throwing: error)
    startup = nil
    listener?.cancel()
    listener = nil
    for job in jobs.values { job.cancel() }
    jobs.removeAll()
    for connection in connections.values { connection.cancel() }
    connections.removeAll()
  }

  private func accept(_ connection: NWConnection) {
    guard listener != nil, connections.count < 8 else {
      connection.cancel()
      return
    }
    let id = UUID()
    connections[id] = connection
    connection.start(queue: .main)
    read(connection, id: id, buffer: Data())
    Task { [weak self] in
      try? await Task.sleep(for: .seconds(70))
      self?.close(id)
    }
  }

  private func close(_ id: UUID) {
    jobs.removeValue(forKey: id)?.cancel()
    connections.removeValue(forKey: id)?.cancel()
  }

  private func read(_ connection: NWConnection, id: UUID, buffer: Data) {
    connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) {
      [weak self] bytes, _, complete, error in
      Task { @MainActor in
        guard let self, self.connections[id] != nil else {
          connection.cancel()
          return
        }
        var accumulated = buffer
        if let bytes { accumulated.append(bytes) }
        do {
          if let request = try CallbackRequest.parse(accumulated) {
            self.respond(request, connection: connection, id: id)
          } else if complete || error != nil {
            self.close(id)
          } else {
            self.read(connection, id: id, buffer: accumulated)
          }
        } catch {
          self.send(
            .object(["error": .string("Invalid request.")]), status: "400 Bad Request",
            connection: connection, id: id)
        }
      }
    }
  }

  private func respond(_ request: CallbackRequest, connection: NWConnection, id: UUID) {
    guard request.method == "POST", request.headers["host"] == authority,
      request.headers["origin"] == nil, request.headers["sec-fetch-site"] == nil,
      request.headers["authorization"] == "Bearer " + token,
      request.headers["content-type"] == "application/json"
    else {
      send(
        .object(["error": .string("Unauthorized runtime connection.")]), status: "403 Forbidden",
        connection: connection, id: id)
      return
    }
    if request.path == "/mcp", let agentTools {
      respondMCP(request, tools: agentTools, connection: connection, id: id)
      return
    }
    if request.path == "/tools" {
      send(
        .object([
          "tools": .array(
            [TonkToolCatalog.definition("tonk_query"), TonkToolCatalog.definition("tonk_guide")]
              + (allowsWrites ? [TonkToolCatalog.definition("tonk_evaluate")] : [])
              + (supportsInspection ? [SpaceTools.inspectionDefinition] : []))
        ]), connection: connection, id: id)
      return
    }
    guard request.path == "/call",
      let value = try? JSONDecoder().decode(JSONValue.self, from: request.body),
      case .object(let fields) = value, Set(fields.keys) == ["name", "arguments"],
      let name = value["name"].string
    else {
      send(
        .object(["error": .string("Invalid runtime operation.")]), status: "400 Bad Request",
        connection: connection, id: id)
      return
    }
    jobs[id] = Task { [weak self] in
      guard let self else { return }
      let result: JSONValue
      do {
        if name == "tonk_evaluate" && !allowsWrites {
          throw HarnessError.message("This runtime connection is read-only.")
        }
        if name == "tonk_guide" {
          send(
            .object(["result": try TonkAgentContext.guide(value["arguments"])]),
            connection: connection, id: id)
          return
        }
        if name == "tonk_inspect_view" && supportsInspection {
          _ = try SpaceTools.validate(tool: name, arguments: value["arguments"])
        } else if !TonkToolCatalog.primitiveNames.contains(name) {
          throw HarnessError.message("Tool unavailable.")
        }
        result = .object(["result": try await handler(name, value["arguments"])])
      } catch {
        let detail =
          (error as NSError).userInfo["WKJavaScriptExceptionMessage"] as? String
          ?? error.localizedDescription
        result = .object(["error": .string(String(detail.prefix(2000)))])
      }
      guard !Task.isCancelled, connections[id] != nil else { return }
      send(result, connection: connection, id: id)
    }
  }

  private func respondMCP(
    _ request: CallbackRequest, tools: [JSONValue], connection: NWConnection, id: UUID
  ) {
    guard let message = try? JSONDecoder().decode(JSONValue.self, from: request.body),
      message["jsonrpc"].string == "2.0", let method = message["method"].string
    else {
      send(.null, status: "400 Bad Request", connection: connection, id: id)
      return
    }
    let requestID = message["id"]
    if requestID == .null {
      send(.null, status: "202 Accepted", connection: connection, id: id)
      return
    }
    jobs[id] = Task { [weak self] in
      guard let self else { return }
      let result: JSONValue
      switch method {
      case "initialize":
        result = .object([
          "protocolVersion": .string("2025-03-26"),
          "capabilities": .object(["tools": .object([:]), "resources": .object([:])]),
          "instructions": .string(TonkAgentContext.instructions),
          "serverInfo": .object(["name": .string("tonk"), "version": .string("1.0")]),
        ])
      case "ping": result = .object([:])
      case "resources/list": result = .object(["resources": .array(TonkAgentContext.resources)])
      case "resources/read":
        do { result = try TonkAgentContext.resource(message["params"]["uri"].string ?? "") } catch {
          send(
            .object([
              "jsonrpc": .string("2.0"), "id": requestID,
              "error": .object([
                "code": .number(-32602), "message": .string("Unknown guide resource."),
              ]),
            ]),
            connection: connection, id: id)
          return
        }
      case "tools/list": result = .object(["tools": .array(tools)])
      case "tools/call":
        let name = message["params"]["name"].string ?? ""
        guard tools.contains(where: { $0["name"].string == name }) else {
          send(
            .object([
              "jsonrpc": .string("2.0"), "id": requestID,
              "error": .object(["code": .number(-32602), "message": .string("Tool unavailable")]),
            ]),
            connection: connection, id: id)
          return
        }
        do {
          let response = try await handler(name, message["params"]["arguments"])
          // Existing host tools return Codex-style success/content; MCP uses isError.
          result = .object([
            "content": .array(
              response["contentItems"].array.map {
                .object(["type": .string("text"), "text": $0["text"]])
              }), "isError": .bool(response["success"].bool != true),
          ])
        } catch {
          result = .object([
            "content": .array([
              .object([
                "type": .string("text"),
                "text": .string(error.localizedDescription),
              ])
            ]), "isError": .bool(true),
          ])
        }
      default:
        send(
          .object([
            "jsonrpc": .string("2.0"), "id": requestID,
            "error": .object(["code": .number(-32601), "message": .string("Method unavailable")]),
          ]),
          connection: connection, id: id)
        return
      }
      guard !Task.isCancelled, connections[id] != nil else { return }
      send(
        .object(["jsonrpc": .string("2.0"), "id": requestID, "result": result]),
        connection: connection, id: id)
    }
  }

  private func send(
    _ value: JSONValue, status: String = "200 OK", connection: NWConnection, id: UUID
  ) {
    guard let body = try? JSONEncoder().encode(value) else {
      close(id)
      return
    }
    var response = Data(
      "HTTP/1.1 \(status)\r\nContent-Type: application/json\r\nContent-Length: \(body.count)\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
        .utf8)
    response.append(body)
    connection.send(
      content: response,
      completion: .contentProcessed { [weak self] _ in
        Task { @MainActor in self?.close(id) }
      })
  }
}
