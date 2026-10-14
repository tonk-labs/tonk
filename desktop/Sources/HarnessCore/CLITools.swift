import Foundation

public enum CLITools {
  public static let definition = TonkToolCatalog.definition("tonk_cli")

  public static func arguments(_ value: JSONValue) throws -> [String] {
    guard case .object(let fields) = value, let operation = fields["operation"]?.string else {
      throw HarnessError.message("Provide a CLI operation.")
    }
    func allow(_ keys: Set<String>) throws {
      guard Set(fields.keys).isSubset(of: keys.union(["operation"])) else {
        throw HarnessError.message(
          "Unexpected CLI argument; executable, paths and target spaces are fixed by the harness.")
      }
    }
    func target(_ key: String) throws -> String {
      guard let text = fields[key]?.string, !text.isEmpty, text.count <= 200,
        text.range(of: "^[A-Za-z][A-Za-z0-9_:/.-]*$", options: .regularExpression) != nil
      else {
        throw HarnessError.message("Provide a simple concept, entity, or guide name.")
      }
      return text
    }
    switch operation {
    case "guide":
      try allow(["target"])
      let name = try target("target")
      guard ["notation", "views", "events", "glossary"].contains(name) else {
        throw HarnessError.message("Available guides: notation, views, events, glossary.")
      }
      return ["help", name]
    case "show", "query":
      try allow(["target"])
      var args = ["--space", "attached", operation]
      if fields["target"] != nil || operation == "query" { args.append(try target("target")) }
      return args
    case "preview", "apply":
      try allow(["document", "home"])
      guard let document = fields["document"]?.string,
        !document.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
        document.utf8.count <= 32000,
        document.range(of: "!(?!:)|%TAG", options: .regularExpression) == nil
      else {
        throw HarnessError.message(
          "Provide up to 32 KB of inline notation. YAML tags and includes are unavailable; only assertion suffix !: is allowed."
        )
      }
      var args = ["--space", "attached", "eval", "-c", document, "--quiet", "--json"]
      if fields["home"] != nil { args += ["--home", try target("home")] }
      if operation == "preview" { args.append("--dry-run") }
      return args
    default: throw HarnessError.message("Unknown CLI operation.")
    }
  }

  public static func summarize(_ output: String) -> String {
    // eval returns bulky revision signatures. Keep the commit outcome and warnings.
    if let start = output.firstIndex(of: "{"), let end = output.lastIndex(of: "}"),
      let data = String(output[start...end]).data(using: .utf8),
      let value = try? JSONDecoder().decode(JSONValue.self, from: data), value["commits"] != .null
    {
      let result: JSONValue = .object([
        "commits": value["commits"],
        "revisionChanged": .bool(value["revision_before"] != value["revision_after"]),
      ])
      if let data = try? JSONEncoder().encode(result) {
        return String(output[..<start]) + String(decoding: data, as: UTF8.self)
          + String(output[output.index(after: end)...])
      }
    }
    let limit = 48000
    return output.count > limit
      ? String(output.prefix(limit)) + "\n[Truncated by the harness]" : output
  }
}
