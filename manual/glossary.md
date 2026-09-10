# Glossary

> Names matter. Most confusion in a distributed system is two things sharing one name, or one thing carrying two. These are the words we use, and what each one is not.

## Where things live

### Repository

An identity, not a place. A repository is a dialog database identified by a [did:key] minted at creation. It is the thing every [replica] is a replica *of*, and it exists nowhere in particular: you never hold a repository, only a replica of it.

### Space

The conceptual model of which a [repository] is the implementation. A space is what users create, join, share, and name; a repository is how the system realizes one. Same [did:key], different level of description.

### Device

A machine and what runs on it: this laptop, that phone. A device is where [profile]s live, the way a browser is where browser profiles live, and one device can hold several. Device is about place. It holds no authority of its own; the profile does.

### Replica

A [repository] as materialized under one [profile]. May be partial; a profile holds what it has pulled, not the whole. Named by the pair (profile, repository), so the same profile is the same replica wherever it is opened, and two profiles looking at one repository are two replicas. Facts local to this copy of a repository (its remotes, whether sync is paused) attach here.

### Origin

An [operator] on a specific [branch]. Dialog names every revision by the pair (origin, edition), so an origin is a single sequential writer: it produces revisions one at a time and never two with the same edition. The same operator on two branches is two origins; two operators on one branch are two origins. It is not the ([repository], [profile]) pair; that is a [replica]. And it is not the browser's origin (scheme, host, port), which scopes storage and has no standing in this vocabulary.

### Branch

A named, independently versioned line of history within a [replica]. Every repository has `main`, the shared content that syncs between replicas, and `meta`, which never syncs and describes the [replica]: the branches it has, the remotes it tracks, whether sync is paused. Anything that must reach other replicas goes on `main`, never on `meta`.

### Remote

A named upstream a [replica] syncs with: an address plus the repository it serves. A local branch that follows a remote branch is a *tracking branch*. Pull brings the remote's revision here; push sends ours there.

### Overlay

The ephemeral region of a [branch]: facts that live for the session and never commit. Transient concepts, per-tab state, in-flight status, secrets that must not replicate. An overlay fact is visible to queries and subscriptions under this profile and to no one else, ever.

## Who acts

### Account

The user's authority across devices. Capabilities are granted to the account; linking a [profile] delegates a subset of them to it, and unlinking is revoking that delegation. It is the unit of registration and billing. What an account *is*, underneath, is a secret; how that secret is kept is [custody].

### Device profile

The user's authority on a [device]. A profile is a local identity in the sense a browser profile is: its own storage, its own credential, switchable, and a device can hold several. "Profile" for short. Its powers are a subset delegated from the [account], and the account can take them back. A profile is a credential, not a person, and the code calls it exactly this.

### Device link

The `account → profile` delegation itself, as recorded. Being signed in *is* this delegation existing and verifying; signing out revokes and retracts it. There is no separate signed-in flag, because a flag can disagree with the proof and the proof cannot disagree with itself.

### Operator

The credential that actually signs. An operator holds a short-lived delegation from a [profile]; that delegation, not any derivation, is what makes it one. In practice it is derived from the profile by a fixed context so it never moves, but nothing requires that. Every request a profile makes is signed by its operator, and the profile-to-operator delegation is the last hop of every proof chain it presents. That hop is where time lives: the profile is unexpiring; the operator's lease is not. Refusing to renew it is how authority is withdrawn from a device that cannot be reached.

### Session

The identity a query or commit runs under: a [profile], its [operator], and the active [branch]es. One session may span several branches when an [overlay] is in play.

### Member

An [account] on the roster of a [space]. Membership is recorded on `main`, so the roster converges across every replica, and it is keyed on the account rather than any [profile], so one person's row is the same row from every device. A member has a role: founder for the creator, member for anyone who joined, admin for a member promoted to run the space. Before sign-up, the onboarding account stands in, so a membership still names an account and never a device credential.

### Authorization

The public half of an [invitation]: a delegation chain granting access to a [repository] to a principal minted for that invite. A delegation chain is a scoped capability, not a secret, so it is recorded durably and replicates like any fact. It says what the bearer may do; it does not say who the bearer is.

### Membership credential

The private half of an [invitation]: the secret of the principal the [authorization] was minted for. Holding it is what lets a stranger exercise that authorization, so it is a bearer secret: anyone with the link can join. It is never recorded durably; it lives in the minting profile's [overlay] and travels only inside the invite link. It admits, it does not identify: redeeming it proves the delegation, and the redeemer's own [account] is what lands on the roster as a [member].

### Invitation

An [authorization] joined with its [membership credential]. The invite link is the two serialized together. Redeeming it makes the redeemer's account a [member].

## How the secret is kept

### Custody

The arrangement by which an [account]'s secret is held and recovered. The account is a random 32-byte secret; every signing and encryption key derives from it, and every way of keeping it is an interchangeable wrapping of that same secret. Open any one wrapping and you have the whole account. Custody is separate from authority: a [device link] says a profile may act for the account; custody says how the account itself can be reconstituted.

### Envelope

One wrapping of the account secret: ciphertext sealed to a recipient key, binding recipient and subject so it cannot be re-pointed. Each custody method is an envelope. Sealing is randomized, so the same secret sealed twice is two envelopes, and rotation retracts the old one rather than overwriting it.

### Passkey

A WebAuthn credential used as a custody method. Its PRF output derives a custody key and a wrapping key deterministically, and the account secret is sealed in an [envelope] to that custody key. A passkey does not hold the account; it can open the envelope that does. The account is recoverable on any device that can present one of its passkeys, and one account may have several.

### Custodian

The interim keeper. Before an account has any [passkey], it is *onboarding*: a locally generated account whose secret is sealed under a key that lives only on this device. Nothing can recover it elsewhere. Signing up rotates every custodied seed onto the real account and retires the onboarding one.

### Sealed inbox

An address, not a store: the public key anything sealed *for* an [account] is aimed at. Any device can deposit; none can read without a [ceremony]. This is how a space's key reaches a newly linked device without any device ever holding the account secret in the clear.

### Sealed principal

A keypair whose seed is kept sealed so the principal can be re-issued: a space's own key, sealed to the account's [sealed inbox]. The delegation says who may act for the space; the sealed seed says how to mint that delegation again.

## Who pays

### Customer

An [account] as the access service knows it: a DID that enrolled an address, confirmed it, and is served. Registered means enrolled with the link unopened; active means confirmed; suspended means the service withdrew. Nothing is served for a customer that is not active, including its own account space.

### Consumer

A subject the service serves on someone's behalf: in practice a [space] whose sync a [customer] pays for. A consumer is servable while its provider is an active customer, and not otherwise. An account is its own consumer, so there is no special case for account spaces. The access service records this relationship in a table it calls `subscription`, which shares a name with the reactive [subscription] and nothing else.

### Provider

Two relationships, one word. To a [consumer], the provider is the [customer] paying for it: exactly one, required, and the account whose data it is. To an [account], the provider is the access service it is a customer of. A space's [remote] usually points at the account's provider, but they are different facts about different subjects.

## What is known

### Fact

The unit of information: *the* attribute *of* an entity *is* a value, plus a *cause* that says who produced it and orders it in time. Facts are immutable and nothing is lost. An assertion adds a fact; a retraction adds a fact that evicts an earlier one from the current view, while history keeps both. A [repository] is an accretion of facts, not a place that gets overwritten.

### Entity

A stable name for a thing, spelled as a URI. Some entities are keys (a [profile], a [repository]); some are hashes of a description, so two parties describing the same thing independently arrive at the same name; some are plain identifiers (`uuid:`, `site:`). The scheme is a detail. The stability is the point.

### Attribute

A relation with invariants. The bare relation is a `domain/name` pair naming a kind of association; an attribute adds what type of value it admits and how many. Attributes are the vocabulary [concept]s are composed from.

### Concept

A composition of [attribute]s sharing an entity: the shape of a thing, described by its relations. A concept is a lens, not a container. Querying one composes matching [fact]s into a [conclusion]; asserting one decomposes into the individual facts. Schema on read: facts exist whether or not a concept describes them, and many concepts may describe the same entity. A concept's identity is the set of attributes it requires, not the names it gives them.

### Conclusion

A [concept] realized: one entity's matching [fact]s assembled into the shape the concept describes. Conclusions are what queries return and what views render. They are derived, never stored.

### Rule

A way to say that some [conclusion]s follow from others. A rule's body is a set of premises; its head is a concept. Rules are one-directional, because a conclusion may aggregate what it was derived from and cannot be unwound. There are two kinds, and they answer different questions.

#### Deductive rule

Says what is *implied*. A deductive rule defines a relation in terms of other relations and is evaluated at query time; nothing is stored. Ask, and the answer is derived from the facts as they stand now. Use it for anything that is a consequence of current state: a total, a membership, a status.

#### Inductive rule

Says what *happens*. An inductive rule has a head and a body, and at least one premise of the body reads a transient concept. That premise is the trigger: the rule fires once, at commit, when the trigger appears, and its head asserts or retracts durable facts. Use it for anything that is a consequence of an event: a counter incremented, a record created. If a rule has no trigger, it was a deduction wearing the wrong clothes.

### Subscription

A query that stays asked. A subscription is a query held open against a [branch]: every commit to that branch re-evaluates it, and only a changed result is delivered. Subscribers see a sequence of [conclusion]s, not a stream of events. There is no "data changed" notification to interpret; there is the next answer, or silence. Views are built on subscriptions, which is why they need no update logic of their own.

## What is shown and done

### View

How a model is shown. A view is a set of templates keyed by facet (`ui`, `directory`, `label`, `title`), attached to the concept it presents. A view is not a page; it is a rendering of facts, driven by a [subscription], so it re-renders when the facts change and does nothing otherwise.

### Command

A request that something be done. A command is a transient [concept]: asserted into the [overlay], consumed once, by a worker handler or as the trigger of an [inductive rule], and swept before the durable commit. Its outcome, if any, is recorded as ordinary facts. Commands are how events enter the system; facts are what remains of them.

### Route

A mapping from a path pattern to the model the shell mounts. Routes are facts on the branch, so a space can declare its own pages. The one exception is the leading space segment of a URL, which is resolved in code, not data, so no fact can redirect a request to a database it should not touch.

### Site

A tab. A site is the per-tab entity the shell mints and the worker stamps with the tab's current path and the [route] it matched. Several tabs under one [profile] are several sites in the same [overlay], distinct by entity. A site is where a page is rendered; it holds no authority and outlives nothing.

[did:key]: https://w3c-ccg.github.io/did-key-spec/
[account]: #account
[attribute]: #attribute
[authorization]: #authorization
[branch]: #branch
[concept]: #concept
[conclusion]: #conclusion
[membership credential]: #membership-credential
[custody]: #custody
[ceremony]: #ceremony
[device link]: #device-link
[envelope]: #envelope
[passkey]: #passkey
[sealed inbox]: #sealed-inbox
[customer]: #customer
[consumer]: #consumer
[device]: #device
[fact]: #fact
[inductive rule]: #inductive-rule
[invitation]: #invitation
[member]: #member
[operator]: #operator
[origin]: #origin
[overlay]: #overlay
[profile]: #device-profile
[remote]: #remote
[replica]: #replica
[repository]: #repository
[route]: #route
[space]: #space
[subscription]: #subscription
