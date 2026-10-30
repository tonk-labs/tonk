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
  /// The agent ChatGPT or Claude connected as, when this key is one.
  var hosted: HostedAgent? = nil

  /// A person's email, known only when they were added by it. Agents have none.
  var email: EmailContactIdentity? { kind == .person ? contact?.emailIdentity : nil }

  /// The key shown and copied; `nil` for a grant that names no recipient.
  var did: String? { contact?.id ?? grants.first?.recipient ?? hosted?.did }

  /// Contacts, grants and hosted agents become one sorted list. A grant whose
  /// recipient is a known contact joins that contact; other grants group by
  /// recipient key, or stand alone when the worker names none. A hosted agent
  /// joins the contact or grant group with its key, or stands as its own row.
  static func entries(
    contacts: [DesktopContact], grants: [AgentConnection], hosted: [HostedAgent] = []
  ) -> [AddressBookEntry] {
    var byAgent = Dictionary(hosted.map { ($0.did, $0) }, uniquingKeysWith: { first, _ in first })
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
      let hostedAgent = byAgent.removeValue(forKey: contact.id)
      let agent =
        !held.isEmpty || hostedAgent != nil
        || contact.labels.contains { $0.caseInsensitiveCompare("Agent") == .orderedSame }
      return AddressBookEntry(
        id: contact.id, kind: agent ? .agent : .person, name: contact.name, contact: contact,
        grants: held, hosted: hostedAgent)
    }
    for (recipient, held) in byRecipient {
      let hostedAgent = byAgent.removeValue(forKey: recipient)
      // The approval page's name, when it set one, else the grant's.
      let title =
        hostedAgent?.name.flatMap { $0.trimmingCharacters(in: .whitespaces).isEmpty ? nil : $0 }
        ?? name(of: held[0])
      rows.append(
        AddressBookEntry(
          id: recipient, kind: .agent, name: title,
          contact: DesktopContact(id: recipient, name: title, labels: []),
          grants: held, hosted: hostedAgent))
    }
    for agent in byAgent.values {
      rows.append(
        AddressBookEntry(
          id: agent.did, kind: .agent, name: agent.displayName,
          contact: DesktopContact(id: agent.did, name: agent.displayName, labels: []),
          grants: [], hosted: agent))
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
      hosted?.name ?? "",
    ].joined(separator: " ")
    return text.localizedCaseInsensitiveContains(search)
  }
}
