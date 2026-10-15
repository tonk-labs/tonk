import Foundation

/// Projection of the worker's fixed, dry-run concept query. No source is executable here.
public enum SpaceSchema {
  public static let query = """
    concept:
      this: ?c
      concept: ?cc
      name: ?name
      description: ?desc
      source: ?source
    """

  public static func summarize(_ data: Data) throws -> String {
    guard data.count <= 2_000_000 else { throw invalid() }
    let value = try JSONDecoder().decode(JSONValue.self, from: data)
    guard case .object(let envelope) = value,
      envelope["revision_before"] != nil, envelope["revision_after"] != nil,
      value["commits"]["claims"] == .number(0),
      value["revision_before"] == value["revision_after"],
      case .array(let blocks) = value["matches_after"], blocks.count == 1,
      blocks[0]["label"].string == "concept", case .array(let rows) = blocks[0]["results"]
    else {
      throw HarnessError.message(
        "Schema response did not confirm an unchanged revision, zero claims and one concept query block."
      )
    }
    var concepts: [JSONValue] = []
    let named = rows.filter { $0["fields"]["name"].string != nil }
    guard rows.allSatisfy({ $0["this"].string != nil && $0["fields"]["source"].string != nil })
    else {
      throw HarnessError.message("Schema row is missing its identity or source.")
    }
    var truncated = named.count > 100
    // Sort before limiting so the same snapshot yields stable context.
    let sorted = named.sorted {
      ($0["fields"]["name"].string ?? "") < ($1["fields"]["name"].string ?? "")
    }
    for row in sorted.prefix(100) {
      guard let name = row["fields"]["name"].string, let entity = row["this"].string,
        let source = row["fields"]["source"].string,
        let bytes = source.data(using: .utf8)
      else {
        throw HarnessError.message(
          "Schema row shape mismatch (name: \(row["fields"]["name"].string != nil), identity: \(row["this"].string != nil), source: \(row["fields"]["source"].string != nil))."
        )
      }
      let descriptor = try JSONDecoder().decode(JSONValue.self, from: bytes)
      let fieldsValue =
        descriptor["kind"].string == "transient"
        ? descriptor["concept"]["with"] : descriptor["with"]
      guard case .object(let fields) = fieldsValue else {
        let shape: String
        if case .object(let object) = descriptor {
          shape = object.keys.sorted().joined(separator: ", ")
        } else {
          shape = "non-object"
        }
        throw HarnessError.message("Schema descriptor has no field map (keys: \(shape)).")
      }
      truncated = truncated || fields.count > 32
      var projected: [JSONValue] = []
      for key in fields.keys.sorted().prefix(32) {
        let field = fields[key]!
        let attribute: JSONValue
        if let text = field["the"].string {
          attribute = .string(String(text.prefix(500)))
          truncated = truncated || text.count > 500
        } else if let domain = field["the"]["domain"].string,
          let keyed = field["the"]["keyed"].string, ["dictionary", "sequence"].contains(keyed)
        {
          attribute = .object([
            "domain": .string(String(domain.prefix(500))), "keyed": .string(keyed),
          ])
          truncated = truncated || domain.count > 500
        } else {
          throw HarnessError.message("Schema field has an unsupported attribute relation.")
        }
        let type = field["as"].string ?? "Value"
        let cardinality = field["cardinality"].string ?? "one"
        guard type.count <= 80, ["one", "many"].contains(cardinality) else { throw invalid() }
        projected.append(
          .object([
            "name": .string(String(key.prefix(200))),
            "attribute": attribute,
            "type": .string(type),
            "cardinality": .string(cardinality),
            "required": .bool(field["optional"] != .bool(true)),
          ]))
        truncated = truncated || key.count > 200
      }
      truncated = truncated || name.count > 200 || entity.count > 500
      concepts.append(
        .object([
          "name": .string(String(name.prefix(200))), "entity": .string(String(entity.prefix(500))),
          "transient": .bool(descriptor["kind"].string == "transient"),
          "fields": .array(projected),
        ]))
    }
    let result: JSONValue = .object([
      "branch": .string("main"),
      "scope": .string("Named concept schemas, including runtime concepts; not record contents."),
      "totalNamedConcepts": .number(Double(named.count)),
      "unnamedConceptsOmitted": .number(Double(rows.count - named.count)),
      "truncated": .bool(truncated),
      "concepts": .array(concepts),
    ])
    let encoded = try JSONEncoder().encode(result)
    guard encoded.count <= 100_000 else {
      throw HarnessError.message("The space schema exceeds this tool's output limit.")
    }
    return String(decoding: encoded, as: UTF8.self)
  }

  private static func invalid() -> HarnessError {
    .message("The worker returned an invalid or oversized read-only schema response.")
  }
}
