import Foundation
import Network

/// One short-lived loopback handoff. The worker, not this transport, verifies the grant.
@MainActor
public final class BrowserCallback {
  private var listener: NWListener?
  private var connections: [UUID: NWConnection] = [:]
  private var startup: CheckedContinuation<URL, Error>?
  private var completion: CheckedContinuation<Data, Error>?
  private var outcome: Result<Data, Error>?
  private var deadline: Task<Void, Never>?
  private let nonce = UUID().uuidString
  private var origin = ""
  public init() {}

  public func start(timeout: Duration = .seconds(300)) async throws -> URL {
    if let outcome {
      _ = try outcome.get()
      throw CallbackError("This sign-in attempt has ended.")
    }
    guard listener == nil else { throw CallbackError("Sign-in is already waiting.") }
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
          guard let port = server?.port, self.startup != nil else { return }
          self.origin = "http://127.0.0.1:\(port.rawValue)"
          self.startup?.resume(returning: URL(string: self.origin + "/")!)
          self.startup = nil
        case .failed(let error): self.finish(.failure(error))
        default: break
        }
      }
    }
    deadline = Task { [weak self] in
      do { try await Task.sleep(for: timeout) } catch { return }
      self?.finish(.failure(CallbackError("Tonk sign-in timed out. Try again.")))
    }
    return try await withCheckedThrowingContinuation { continuation in
      startup = continuation
      server.start(queue: .main)
    }
  }

  public func receive() async throws -> Data {
    if let outcome { return try outcome.get() }
    return try await withCheckedThrowingContinuation { completion = $0 }
  }

  public func cancel() { finish(.failure(CancellationError())) }

  private func finish(_ result: Result<Data, Error>) {
    guard outcome == nil else { return }
    outcome = result
    if let startup {
      if case .failure(let error) = result { startup.resume(throwing: error) }
      self.startup = nil
    }
    completion?.resume(with: result)
    completion = nil
    deadline?.cancel()
    deadline = nil
    listener?.cancel()
    listener = nil
    for connection in connections.values { connection.cancel() }
    connections.removeAll()
  }

  private func accept(_ connection: NWConnection) {
    guard outcome == nil, connections.count < 8 else {
      connection.cancel()
      return
    }
    let id = UUID()
    connections[id] = connection
    connection.start(queue: .main)
    read(connection, id: id, accumulated: Data())
    Task { [weak self, weak connection] in
      try? await Task.sleep(for: .seconds(10))
      connection?.cancel()
      self?.connections.removeValue(forKey: id)
    }
  }

  private func read(_ connection: NWConnection, id: UUID, accumulated: Data) {
    connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) {
      [weak self] bytes, _, complete, error in
      Task { @MainActor in
        guard let self, self.outcome == nil else {
          connection.cancel()
          return
        }
        var buffer = accumulated
        if let bytes { buffer.append(bytes) }
        do {
          if let request = try CallbackRequest.parse(buffer) {
            self.respond(request, on: connection, id: id)
          } else if complete || error != nil {
            connection.cancel()
            self.connections.removeValue(forKey: id)
          } else {
            self.read(connection, id: id, accumulated: buffer)
          }
        } catch {
          self.send("400 Bad Request", body: "Invalid callback request.", on: connection, id: id)
        }
      }
    }
  }

  private func respond(_ request: CallbackRequest, on connection: NWConnection, id: UUID) {
    guard request.path == "/", request.headers["host"] == String(origin.dropFirst(7)) else {
      send("404 Not Found", body: "Not found.", on: connection, id: id)
      return
    }
    if request.method == "GET" {
      send(
        "200 OK",
        body: """
          <!doctype html><meta charset="utf-8"><meta name="referrer" content="no-referrer">
          <title>Tonk</title><p id="status">Returning to Tonk…</p>
          <script>
          const fields = new URLSearchParams(location.hash.slice(1));
          history.replaceState(null, '', '/');
          if (fields.has('authorize') || fields.has('deny')) {
            const form = document.createElement('form'); form.method = 'post'; form.action = '/';
            for (const name of ['authorize', 'deny']) {
              if (!fields.has(name)) continue;
              const input = document.createElement('input'); input.type = 'hidden';
              input.name = name; input.value = fields.get(name); form.appendChild(input);
            }
            const nonce = document.createElement('input'); nonce.type = 'hidden';
            nonce.name = 'state'; nonce.value = '\(nonce)'; form.appendChild(nonce);
            document.body.appendChild(form); form.submit();
          } else { document.querySelector('#status').textContent = 'No authorization received. Return to Tonk to try again.'; }
          </script>
          """, on: connection, id: id)
      return
    }
    // A no-referrer document can serialize its form Origin as null. Fetch Metadata
    // still identifies the same-origin hop; the unguessable bridge token is required too.
    let sameOrigin =
      request.headers["origin"] == origin
      || (request.headers["origin"] == "null" && request.headers["sec-fetch-site"] == "same-origin")
    guard request.method == "POST", sameOrigin,
      request.headers["content-type"]?.hasPrefix("application/x-www-form-urlencoded") == true,
      let fields = request.form, fields["state"] == nonce
    else {
      send("403 Forbidden", body: "Invalid callback origin or state.", on: connection, id: id)
      return
    }
    let result: Result<Data, Error>
    if fields["deny"] != nil {
      result = .failure(CallbackError("Tonk sign-in was declined in the browser."))
    } else if let encoded = fields["authorize"], let bytes = Data(base64Encoded: encoded) {
      result = .success(bytes)
    } else {
      send("400 Bad Request", body: "Invalid authorization.", on: connection, id: id)
      return
    }
    send(
      "200 OK",
      body:
        "<!doctype html><title>Tonk</title><p>Authorization received. Return to Tonk to finish connecting.</p>",
      on: connection, id: id
    ) {
      self.finish(result)
    }
  }

  private func send(
    _ status: String, body: String, on connection: NWConnection, id: UUID,
    then: (() -> Void)? = nil
  ) {
    let response =
      "HTTP/1.1 \(status)\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: \(body.utf8.count)\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\nConnection: close\r\n\r\n\(body)"
    connection.send(
      content: Data(response.utf8),
      completion: .contentProcessed { [weak self] _ in
        Task { @MainActor in
          connection.cancel()
          self?.connections.removeValue(forKey: id)
          then?()
        }
      })
  }
}

public struct CallbackError: LocalizedError {
  public let errorDescription: String?
  public init(_ message: String) { errorDescription = message }
}

struct CallbackRequest {
  let method: String
  let path: String
  let headers: [String: String]
  let body: Data
  static func parse(_ data: Data) throws -> Self? {
    guard data.count <= 262144 else { throw CallbackError("Callback too large") }
    guard let boundary = data.range(of: Data("\r\n\r\n".utf8)) else {
      guard data.count < 16384 else { throw CallbackError("Headers too large") }
      return nil
    }
    guard boundary.lowerBound < 16384,
      let head = String(data: data[..<boundary.lowerBound], encoding: .utf8)
    else { throw CallbackError("Invalid headers") }
    let lines = head.components(separatedBy: "\r\n")
    let first = lines[0].split(separator: " ")
    guard first.count == 3, first[2] == "HTTP/1.1" else { throw CallbackError("Invalid request") }
    var headers: [String: String] = [:]
    for line in lines.dropFirst() {
      guard let colon = line.firstIndex(of: ":") else { throw CallbackError("Invalid header") }
      let key = line[..<colon].lowercased()
      guard headers[key] == nil else { throw CallbackError("Duplicate header") }
      headers[key] = line[line.index(after: colon)...].trimmingCharacters(in: .whitespaces)
    }
    guard headers["transfer-encoding"] == nil,
      let length = Int(headers["content-length"] ?? "0"), length >= 0, length <= 245760
    else { throw CallbackError("Invalid body length") }
    let end = boundary.upperBound + length
    guard data.count >= end else { return nil }
    guard data.count == end else { throw CallbackError("Unexpected trailing data") }
    return Self(
      method: String(first[0]), path: String(first[1]), headers: headers,
      body: Data(data[boundary.upperBound..<end]))
  }
  var form: [String: String]? {
    guard let text = String(data: body, encoding: .utf8) else { return nil }
    var values: [String: String] = [:]
    for pair in text.split(separator: "&") {
      let parts = pair.split(separator: "=", maxSplits: 1, omittingEmptySubsequences: false)
      guard parts.count == 2,
        let key = String(parts[0]).replacingOccurrences(of: "+", with: " ").removingPercentEncoding,
        let value = String(parts[1]).replacingOccurrences(of: "+", with: " ")
          .removingPercentEncoding,
        values[key] == nil
      else { return nil }
      values[key] = value
    }
    return values
  }
}
