import HarnessCore

/// One row of the address book: a person or an agent with access. People and
/// agents are kept in one list so who can reach your spaces is visible in one
/// place. Tonk's own chat agent is an app setting, not a contact.
struct AddressBookEntry: Identifiable, Equatable {
  enum Kind: Equatable {
    case person
    /// Holds access grants under its own key, or is labelled as an agent.
    case agent
  }

  let id: String
  let kind: Kind
  let name: String
  /// The saved or roster contact, when this entry has a key we know.
  let contact: DesktopContact?
  /// Live grants that something connected through, newest first.
  let grants: [AgentConnection]

  /// A person's email, known only when they were added by it. Agents have none.
  var email: EmailContactIdentity? { kind == .person ? contact?.emailIdentity : nil }

  /// The key shown and copied; `nil` for a grant that names no recipient.
  var did: String? { contact?.id ?? grants.first?.recipient }

  /// Contacts and grants become one sorted list. A grant whose recipient is a
  /// known contact joins that contact; other grants group by recipient key,
  /// or stand alone when the worker names none.
  static func entries(
    contacts: [DesktopContact], grants: [AgentConnection]
  ) -> [AddressBookEntry] {
    var byRecipient: [String: [AgentConnection]] = [:]
    var anonymous: [AgentConnection] = []
    for grant in grants.sorted(by: { $0.expiresAt > $1.expiresAt }) {
      if let recipient = grant.recipient {
        byRecipient[recipient, default: []].append(grant)
      } else {
        anonymous.append(grant)
      }
    }
    var rows = contacts.map { contact in
      let held = byRecipient.removeValue(forKey: contact.id) ?? []
      let agent =
        !held.isEmpty
        || contact.labels.contains { $0.caseInsensitiveCompare("Agent") == .orderedSame }
      return AddressBookEntry(
        id: contact.id, kind: agent ? .agent : .person, name: contact.name, contact: contact,
        grants: held)
    }
    for (recipient, held) in byRecipient {
      rows.append(
        AddressBookEntry(
          id: recipient, kind: .agent, name: name(of: held[0]),
          contact: DesktopContact(id: recipient, name: name(of: held[0]), labels: []),
          grants: held))
    }
    for grant in anonymous {
      rows.append(
        AddressBookEntry(
          id: "grant:" + grant.id, kind: .agent, name: name(of: grant), contact: nil,
          grants: [grant]))
    }
    rows.sort { $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    return rows
  }

  /// A client's own name reads better than a generic grant label.
  static func name(of grant: AgentConnection) -> String {
    if !grant.label.isEmpty { return grant.label }
    if let client = grant.installations.first?.name { return client }
    return grant.isTerminalAuthorization ? "Tonk CLI" : "Connected agent"
  }

  func matches(_ search: String) -> Bool {
    guard !search.isEmpty else { return true }
    let text = [
      name, did ?? "", contact?.emailIdentity?.email ?? "",
      grants.flatMap { $0.installations.map(\.name) }.joined(separator: " "),
    ].joined(separator: " ")
    return text.localizedCaseInsensitiveContains(search)
  }
}
