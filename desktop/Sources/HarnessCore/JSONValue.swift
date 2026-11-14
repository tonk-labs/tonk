import Foundation

public enum JSONValue: Codable, Equatable, Sendable {
  case object([String: JSONValue])
  case array([JSONValue])
  case string(String)
  case number(Double)
  case bool(Bool)
  case null

  public init(from decoder: Decoder) throws {
    let container = try decoder.singleValueContainer()
    if container.decodeNil() {
      self = .null
    } else if let value = try? container.decode(Bool.self) {
      self = .bool(value)
    } else if let value = try? container.decode(String.self) {
      self = .string(value)
    } else if let value = try? container.decode(Double.self) {
      self = .number(value)
    } else if let value = try? container.decode([String: JSONValue].self) {
      self = .object(value)
    } else {
      self = .array(try container.decode([JSONValue].self))
    }
  }

  public func encode(to encoder: Encoder) throws {
    var container = encoder.singleValueContainer()
    switch self {
    case .object(let value): try container.encode(value)
    case .array(let value): try container.encode(value)
    case .string(let value): try container.encode(value)
    case .number(let value): try container.encode(value)
    case .bool(let value): try container.encode(value)
    case .null: try container.encodeNil()
    }
  }

  public subscript(_ key: String) -> JSONValue {
    guard case .object(let values) = self else { return .null }
    return values[key] ?? .null
  }
  public var string: String? {
    if case .string(let value) = self { return value }
    return nil
  }
  public var array: [JSONValue] {
    if case .array(let value) = self { return value }
    return []
  }
  public var bool: Bool? {
    if case .bool(let value) = self { return value }
    return nil
  }
  public var int: Int? {
    guard case .number(let value) = self, value.isFinite,
      value >= Double(Int.min), value < Double(Int.max), value.rounded() == value
    else { return nil }
    return Int(value)
  }
}

/// App-server uses newline-delimited JSON, not LSP Content-Length framing.
public struct JSONLines {
  private var buffer = Data()
  public var maximumBytes: Int
  public init(maximumBytes: Int = 16 * 1024 * 1024) { self.maximumBytes = maximumBytes }

  public mutating func append(_ data: Data) throws -> [JSONValue] {
    buffer.append(data)
    var messages: [JSONValue] = []
    while let newline = buffer.firstIndex(of: 10) {
      let line = buffer[..<newline]
      guard line.count <= maximumBytes else {
        throw HarnessError.message("Agent response exceeded the protocol limit.")
      }
      if !line.isEmpty { messages.append(try JSONDecoder().decode(JSONValue.self, from: line)) }
      buffer.removeSubrange(...newline)
    }
    guard buffer.count <= maximumBytes else {
      throw HarnessError.message("Agent response exceeded the protocol limit.")
    }
    return messages
  }
}

public enum HarnessError: LocalizedError {
  case message(String)
  public var errorDescription: String? {
    if case .message(let text) = self { return text }
    return nil
  }
}
