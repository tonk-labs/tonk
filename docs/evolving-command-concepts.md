# Nominal commands and event projections

Status: proposed specification

Date: 2026-08-05

## Summary

Tonk commands are nominal, transient invocations. A command is identified by a
stable command entity; its payload is validated against a separately evolvable
schema. Rules and native handlers dispatch on the command identity before they
inspect its payload.

DOM extraction is not part of a command's schema. A named event projection maps
a browser event into a command invocation and declares synchronous browser
actions such as `prevent-default`. Projections are branch data, can be inspected
and validated without firing an event, and can be exercised headlessly against
an event fixture.

This replaces the current arrangement in which one structural descriptor does
three jobs:

1. it identifies the transient that rules and handlers consume;
2. it declares the command payload; and
3. through `the: dom.event...`, it describes how a browser event supplies that
   payload.

Durable concepts, their semantic `the:` attributes, ordinary facts, queries,
and rules that do not consume commands are unchanged.

## Status words

The terms **MUST**, **MUST NOT**, **SHOULD**, and **MAY** are normative.

## Goals

The design MUST:

- prevent two commands with the same payload shape from cross-firing;
- make a command's identity stable across compatible payload-schema changes;
- keep DOM paths and browser actions out of logical command schemas;
- reject missing required arguments and invalid event projections loudly;
- allow one command to be invoked from multiple UI events;
- allow commands to be invoked without a DOM event;
- make the complete event-to-command projection testable without a browser;
- preserve transient rule semantics and durable fact semantics;
- support an explicit migration of existing profile and spot branches; and
- provide a compatibility window in which legacy spots continue to run.

## Non-goals

This specification does not:

- make durable concepts nominal;
- introduce arbitrary JavaScript into notation;
- define conditional event actions;
- turn command invocation into a durable event log;
- define command authorization independently of the branch transaction's
  existing authority checks;
- define general command-version negotiation; or
- automatically rewrite arbitrary custom elements.

## Terminology

### Command kind

A command kind is the stable entity used for dispatch, for example
`id:todo/add` or `tonk:invite`. It answers “which intent is this?”

### Command schema

A command schema is the structural descriptor that declares the command's
required and optional arguments and their types. It answers “which arguments
does this command accept?” A schema may change while its command kind remains
stable.

### Command invocation

A command invocation is a transient application of a command kind to concrete
arguments. It is visible during the current reactor cycle and is swept before
the durable commit, as command transients are today.

### Event projection

An event projection is a named adapter from a browser event and its bound DOM
element to one command invocation. It declares argument sources and synchronous
event actions. It is not a rule and does not run in the service worker.

### Legacy structural command

A legacy structural command is a current `command!:`/transient concept whose
identity is inferred from its descriptor and whose `the: dom.event...` fields
also serve as an extraction program.

## Required invariants

1. **Dispatch is nominal.** A rule or handler registered for command `A` MUST
   NOT run for command `B`, even when `A` and `B` have identical schemas and
   arguments.
2. **Validation is structural.** After nominal dispatch selects a command, its
   arguments MUST be validated against that command's current schema.
3. **Projection is external to the schema.** A command schema MUST NOT contain
   `dom.event/*` or `dom.event.do/*` attributes.
4. **Actions are not arguments.** Browser actions MUST NOT create command
   fields and MUST NOT participate in rule matching.
5. **Missing required arguments fail.** A command invocation lacking any
   required argument MUST be rejected before it enters the reactor.
6. **Blank is a value.** An empty string supplied to a text argument MUST remain
   an empty string. It MUST NOT be converted into an omitted argument.
7. **Optional means optional.** Only fields declared under `maybe:` may be
   omitted from an invocation.
8. **New and legacy dispatch are isolated.** During compatibility, a nominal
   invocation MUST NOT also be offered to a legacy structural rule or handler.
9. **No partial projection.** If extraction or coercion of a required argument
   fails, the runtime MUST NOT post a partial invocation.
10. **One commit switches a binding.** Installing a replacement rule,
    installing its projections, changing a view binding, and retracting its
    legacy rule MUST be expressible in one notation transaction.
11. **Arguments are not ordinary facts.** A nominal command's arguments MUST
    NOT make its invocation match a durable or structurally transient concept
    that happens to use the same attributes.
12. **Occurrence is not a domain target.** The reserved `this` of a nominal
    command invocation identifies that occurrence. A todo, page, space, or
    other domain target MUST be a named command argument.

## Command declarations

The `command!:` notation head remains. Its `with:` and `maybe:` blocks declare
semantic arguments in exactly the same form as concept fields, but a new
command declaration MUST use ordinary semantic attributes rather than
`dom.event/*` attributes.

```yaml
attribute!: &todo-title
  the: xyz.tonk.todo/title
  as: text

command!: &todo/add
  description: Add a todo.
  with:
    title: todo-title
```

Every command declaration MUST have a stable public identity. Its canonical
command kind is selected as follows:

1. If the declaration has an explicit `this:` URI, that URI is the command
   kind. An `&anchor`, if present, is an alias for it.
2. Otherwise an anchored declaration uses the anchor entity, `id:<anchor>`, as
   its command kind.
3. An unanchored declaration without `this:` is invalid. Content-derived
   identity is unsuitable for a command because changing its schema would
   silently create a different command kind.

The analyzer MUST persist both the command kind and its current schema. The
schema may remain content-addressed; the command kind is the stable reference
to it. Reasserting an anchored command with a changed schema updates that
reference without changing the command kind.

`tonk schema` MUST render nominal commands as `command!:` and MUST preserve
their identity when its output is resubmitted.

### Schema evolution

Adding an optional argument under `maybe:` is backward compatible. Removing an
optional argument, changing an argument's type, renaming an argument, or adding
a required argument is incompatible with existing callers and requires a
migration of their projections or programmatic invocations.

The runtime does not negotiate schema versions in V1. It resolves the current
schema for the command kind and validates the invocation against it. A future
version may add explicit schema revisions without changing nominal dispatch.

Native Rust decoders SHOULD represent a `maybe:` argument as `Option<T>`. They
SHOULD NOT recover a declared optional argument with an untyped raw-fact reader;
doing so hides the compatibility obligation from the handler's type.

### Existing compatibility history

This schema-evolution problem predates nominal commands. `CreateSpace.remote`
could not become a required Rust concept field because older profile branches
had a name-only descriptor. `Invite.space` encountered the same constraint:
older spot descriptors were frozen without the field, so a new required field
would let the transient commit while selecting no handler. Both cases were
worked around by reading newer attributes opportunistically from raw facts.

Those workarounds remain necessary while legacy structural commands exist. A
legacy-compatible new field belongs under `maybe:` and in an `Option<T>` Rust
field where its type can round-trip. Migration to a nominal command moves the
complete argument contract into the command schema, and the nominal handler
MUST use the typed argument rather than the raw-fact reader. A separate legacy
adapter MAY retain the reader until every persisted caller is migrated and the
compatibility runtime is removed. Nominality stabilizes command identity; it
does not make incompatible required-field changes backward compatible by
itself.

## Rules

The rule surface remains recognisable:

```yaml
rule!:
  this: effect:todo-add
  assert!: todo
  when:
    - assert: todo/add
      where: { title: ?title }
  where:
    title: ?title
```

When a premise resolves to a command, the analyzer MUST compile the command
kind into the installed rule in addition to its argument terms. At runtime the
reactor MUST first select rules by command kind, then unify their argument
terms.

Consequences:

- another command with a `title` argument cannot fire this rule;
- a rule may match a subset of optional arguments, as ordinary unification
  permits;
- a rule premise naming a required argument is guaranteed that the invocation
  passed command validation; and
- command-to-command rules remain possible: a rule head naming a command emits
  a nominal invocation directly and requires no event projection.

Rules that consume only durable concepts retain structural matching. A
transient `concept!:` that is not declared as a command also retains existing
structural semantics. Nominal command invocations are not visible to those
legacy structural premises during the compatibility period.

Logically, the reactor evaluates command premises against a transient command
input relation keyed by `(command kind, occurrence, arguments)`. An
implementation MAY encode that relation inside the transaction overlay using
reserved `dialog.command/*` relations so the existing query engine can join it
to durable premises. It MUST NOT encode arguments under their semantic concept
attributes, publish a concept descriptor for the occurrence, expose the
reserved relations through ordinary concept resolution, or select consumers
from argument shape. The kind index selects command consumers first; reserved
argument relations are only the private unification representation for those
selected consumers. Other premises in the same rule continue to query durable
and structurally transient facts normally, and may join their values with
variables bound by the command premise.

Each `invoke` claim is one occurrence. Two identical claims are two command
occurrences and each is eligible to fire its selected consumers once. The
reactor assigns an occurrence entity when it receives the claim. A rule may
bind that entity through the command premise's reserved `this` term, but it is
not a declared argument or durable fact. Native handlers receive it as command
metadata.
Client-supplied retry/idempotency keys are outside V1.

`invoke.arguments` MUST NOT contain `this`. A command that acts on a domain
entity declares an explicit argument such as `todo`, `page`, or `space`.
Legacy commands that overloaded their transient subject as the target must
rename that input during migration.

An externally invoked command seeds the current effect round. A command emitted
by a rule head becomes a new occurrence in the next round. Command occurrences
are swept after their round; they never enter the durable commit. Existing
maximum-round protection applies to command-to-command cycles and aborts the
whole transaction if the fixpoint does not terminate.

New command-consuming rules SHOULD use stable `this:` entities. This makes a
later migration able to address and retract the installed rule without first
discovering a content-derived `effect:` entity.

## Native handlers

The native command-handler registry MUST use the same command kind as the rule
engine. A handler registration consists of:

- the command kind;
- the Rust argument decoder; and
- the handler implementation.

The kind selects the handler. The decoder then validates or decodes its
arguments. Descriptor compatibility alone MUST NOT select a handler.

The handler API SHOULD expose the origin repository and branch separately from
command arguments, as it does today. Transport context is not part of a command
schema unless it is intentionally supplied as an argument.

Native handlers run after the triggering transaction commits. Consumer lookup
MUST confirm before commit that the command kind has a registered handler, but
the handler's asynchronous I/O and any commits it performs are not part of the
triggering transaction. The transaction response reports scheduled handlers,
not completed handlers.

Each scheduled handler receives the invocation correlation identifier and MUST
publish a structured completion or failure outcome under that identifier.
Handler failure cannot roll back the triggering commit or a sibling handler's
work. Declarative rules remain inside the triggering transaction and a rule
evaluation failure aborts that transaction.

## Event projection notation

`projection!:` is a new declaration head. It is stored on the branch and has a
name, a target command, an argument-source map, and zero or more event actions.

```yaml
projection!: &todo/add-form
  command: todo/add
  default: true
  arguments:
    title:
      control: title
  actions:
    - prevent-default
```

The fields are:

- `command` — required command name or command-kind URI;
- `default` — optional boolean, false when absent;
- `arguments` — command field name to source expression; and
- `actions` — optional ordered list of synchronous browser actions.

A projection declaration MUST be anchored or have an explicit `this:` URI.
Projection identity is independent of command identity, so a command may have
several projections.

The analyzer MUST reject a projection when:

- `command` does not resolve to a nominal command;
- an `arguments` key is not declared by the command schema;
- a required command argument has no source;
- a source form or source option is unknown;
- an action is unknown;
- more than one projection for a command declares `default: true`; or
- the projection and command form a name-resolution cycle.

An optional command argument may be absent from `arguments`. If it has a source
but that source is missing at runtime, the argument is omitted and a trace
event is emitted.

### V1 source expressions

V1 supports the following source forms.

#### Named form control

```yaml
title: { control: title }
```

The runtime finds a form as follows:

1. if the bound element is a form, use it;
2. otherwise use the bound element's `form` property; and
3. resolve the control with `form.elements.namedItem(name)`.

The scalar form reads the control's `value`. The expanded form may select
`value` or `checked`:

```yaml
done:
  control:
    name: done
    property: checked
```

Control names are exact strings. They are not camel-cased, so `note-body`
resolves the control whose name is exactly `note-body`.

#### Bound-element data attribute

```yaml
todo: { data: todo }
```

This reads `data-todo` from the element carrying the `on<event>` binding using
`getAttribute`. A name such as `todo-id` reads `data-todo-id` exactly.

#### Event member

```yaml
time: { event: timeStamp }
```

This reads one public scalar member from the event object. V1 permits a single
member, not an arbitrary property path. The implementation maintains an
explicit registry of supported members, initially including `type`, `key`,
`code`, `repeat`, `shiftKey`, `ctrlKey`, `altKey`, `metaKey`, `button`,
`clientX`, `clientY`, and `timeStamp`. The analyzer rejects names outside the
registry.

#### Custom-event detail member

```yaml
sheet: { detail: sheet }
```

This reads one member from `CustomEvent.detail`. V1 permits a single member.
Custom elements SHOULD expose stable, command-oriented detail names rather than
DOM implementation objects.

#### Event target member

```yaml
value: { target: value }
```

This reads one scalar member from the event's target. V1 supports `value` and
`checked`. It is intended for input and change events. Context authored on the
binding element belongs in `data:`, not `target:`.

#### Literal

```yaml
direction: { literal: next }
```

This supplies the scalar literal unchanged before command-schema coercion.

V1 deliberately has no generic property-walk source. A future `path:` escape
hatch may be added, but it must be marked opaque to static validation and must
not change the semantics of the typed forms above.

### V1 event actions

V1 defines:

- `prevent-default`;
- `stop-propagation`; and
- `stop-immediate-propagation`.

The runtime MUST extract and coerce every required argument before executing
actions. Once a valid invocation has been constructed, it MUST execute actions
synchronously, in declaration order, before posting the invocation. A later
network or worker failure cannot undo an event action and must be reported
separately.

Actions are not represented in the command application sent over the wire.

## Template binding and projection resolution

The HTML surface remains `on<event>=<reference>`.

```html
<form onsubmit=todo/add>
  <input name="title">
  <button type="submit">Add</button>
</form>
```

The runtime resolves the right-hand side in this order:

1. If it names an event projection, use that projection.
2. If it names a command with exactly one projection, use that projection.
3. If it names a command with one projection marked `default: true`, use the
   default projection.
4. If it names a command with multiple projections and no unique default,
   report an ambiguous-projection error and do not bind the event.
5. During compatibility only, if it names a legacy structural command with no
   projection, use legacy `dom.event` extraction.
6. Otherwise report an unresolved-event-binding error and do not bind it.

This permits existing `onclick=todo/add` markup to remain unchanged when the
migration installs one unambiguous projection. A view names a projection
explicitly only when it needs a non-default adapter.

A bare `onclick`, an unresolved reference, or an ambiguous reference MUST be
reported. These cases MUST NOT remain silent no-ops.

## Runtime projection algorithm

For each rendered `on<event>` binding, the display runtime resolves and caches
the projection and its target command schema. When the event fires it MUST:

1. read every declared source from the live event and bound element;
2. retain empty strings and other valid false-like scalar values;
3. coerce each supplied value according to the command schema;
4. omit only missing optional arguments;
5. abort and report the exact source and field if a required source is missing
   or coercion fails;
6. construct a nominal command invocation;
7. execute the projection's event actions synchronously; and
8. post the invocation to `/transact`.

Projection resolution errors belong to render/bind time. Extraction and
coercion errors belong to event-fire time. Transaction and dispatch errors
belong to the worker response. The runtime MUST preserve that distinction in
its diagnostics.

The cache MUST be invalidated when the branch's command, projection, or name
claims change. Editing a projection in the inspector must affect the next
render or event binding without recreating the spot.

The projection evaluator SHOULD be a source-independent core with two adapters:

- a browser adapter that implements the typed sources over a live Event and
  bound Element; and
- a fixture adapter that implements the same typed sources over serializable
  test data.

Extraction order, blank handling, optional handling, type coercion, and error
classification belong in the shared evaluator rather than either adapter.

## Wire protocol

`/transact` gains a command-specific source claim. The normative JSON shape is:

```json
{
  "claims": [
    {
      "op": "invoke",
      "command": "id:todo/add",
      "arguments": {
        "title": "Buy milk"
      }
    }
  ]
}
```

`invoke` accepts only a nominal command kind. It cannot invoke a durable
concept. The worker MUST:

1. resolve the command kind against the transaction's branch;
2. reject an unknown command;
3. load its current schema;
4. reject unknown arguments;
5. reject missing required arguments;
6. coerce or reject supplied values using existing typed-application rules;
7. create one occurrence in the transient command input relation; and
8. evaluate only rules and handlers registered for that kind and occurrence.

`SourceApplication.name` is not command identity. It retains its current
meaning of publishing an asserted entity under an anchor. The implementation
MUST introduce a separate command field/type rather than overloading `name`.

Existing `assert` and `retract` source claims remain available for durable
concept applications and legacy compatibility. Once a command has been
migrated, clients SHOULD use `invoke` rather than posting its full predicate.

The response MUST distinguish at least:

- command accepted and one or more rules fired;
- command accepted and one or more native handlers were scheduled;
- command accepted, rules are registered for its kind, but no rule's full body
  matched;
- command accepted but no rule or handler is installed;
- command rejected during resolution or validation; and
- transactional rule evaluation failed.

Command resolution, argument validation, and consumer lookup occur before the
durable transaction commits. A request containing an unknown, invalid, or
unhandled command—one with no rule or handler registered for its kind—MUST fail
atomically. It MUST NOT return a successful commit with zero registered
consumers. A registered rule whose non-command premises do not match is a
handled no-effect outcome, not an unhandled command. A registered native handler
counts as a consumer during preflight and is scheduled only after a successful
commit.

A successful response adds one outcome per invocation to the existing commit
summary. The normative additional shape is:

```json
{
  "invocations": [
    {
      "claim": 0,
      "command": "id:todo/add",
      "status": "handled",
      "registered_rules": 1,
      "fired_rules": 1,
      "registered_handlers": 0,
      "scheduled_handlers": 0,
      "correlation": "request-local opaque identifier"
    }
  ]
}
```

Validation and unhandled-command responses use the existing structured error
envelope and include `claim`, `command`, and a stable error code. Consumer
lookup and rule-evaluation failure abort the transaction and identify the
selected consumer without including private argument values. Native-handler
completion or failure is reported asynchronously under `correlation`; it does
not change the already-returned commit response.

The occurrence identifier is assigned by the receiving runtime and does not
appear in the normative V1 request body. It distinguishes repeated identical
claims within command evaluation. It does not make network retries idempotent.

## Implementation boundaries

The logical separation in this specification must remain visible in code:

- notation and analysis parse `projection!:` and resolve stable command kinds;
- core claim types represent `invoke` separately from predicate `assert` and
  `retract`;
- stored rule metadata carries command kinds for command premises and heads;
- the reactor owns the transient command input relation and occurrence loop;
- the native handler registry selects by command kind;
- the display runtime owns DOM and CustomEvent adapters plus synchronous event
  actions;
- a source-independent projection evaluator owns extraction semantics and is
  shared by the display runtime and CLI fixture adapter;
- the worker resolves command schemas authoritatively and reports invocation
  outcomes; and
- schema/introspection surfaces round-trip both command and projection
  declarations.

The implementation may choose internal artifact attribute names and indexes,
but those choices MUST preserve stable command identity, schema replacement,
branch-local projection resolution, and exact rule retraction.

## Programmatic invocation

Custom elements and application code may invoke a command directly without an
event projection. The public API SHOULD mirror the wire model:

```javascript
await tonk.invoke("id:todo/add", { title: "Buy milk" })
```

Programmatic invocation performs the same branch resolution, validation,
dispatch, and diagnostics as a projected invocation. It MUST NOT accept a
caller-supplied predicate as a substitute for command identity.

A custom element may instead emit a `CustomEvent` and use a projection with
`detail:` sources. The two paths are equivalent after projection.

## Diagnostics and observability

Every invocation receives a correlation identifier for tracing. This is
diagnostic metadata, not command occurrence identity and not a command
argument.

The main-thread runtime MUST emit structured diagnostics for:

- unresolved binding;
- ambiguous projection;
- missing required source;
- missing optional source;
- source read failure;
- coercion failure; and
- transaction request or response failure.

The worker MUST emit structured diagnostics for:

- unknown command kind;
- schema-resolution failure;
- unknown argument;
- missing required argument;
- rule/handler selection count;
- no installed consumer; and
- transactional rule failure;
- native handler scheduled; and
- native handler completed or failed.

At minimum, each record includes the correlation identifier, command kind,
projection identity when present, field/source when relevant, repository,
branch, and outcome.

Development builds MUST surface binding and event-fire failures in the browser
console. The inspector SHOULD show the last invocation outcome for a selected
binding. Production telemetry remains subject to the repository's telemetry
policy and MUST NOT record sensitive argument values by default.

## Headless projection verification

The CLI MUST provide a verifier that executes the real projection code against
a serializable event fixture. It must not synthesize command arguments directly.

The conceptual interface is:

```bash
tonk project todo/add-form \
  --spot recipe-tracker \
  --fixture fixtures/add-todo-event.yaml
```

The result MUST report:

- the resolved projection and command kind;
- every extracted argument and its source;
- omitted optional arguments;
- planned event actions;
- the exact `invoke` request, with values redacted on request; and
- validation errors.

The verifier MUST use the same source readers, blank-value rules, and coercion
logic as the browser runtime. A fixture describes the semantic inputs used by
the typed source forms—controls, data attributes, event members, detail,
target, and literals—not an arbitrary mock JavaScript object.

`--transact` MAY submit the successfully projected invocation to a disposable
or explicitly selected spot. The default is non-mutating.

## Compatibility

Compatibility is a runtime mode, not a permanent second command model.

During the migration window:

- existing structural `command!:` declarations remain resolvable as legacy
  commands, including handler-consumed commands whose attributes were already
  semantic rather than `dom.event/*`;
- existing event bindings may use legacy extraction when no projection exists;
- existing full-predicate `assert` transactions remain accepted;
- nominal `invoke` transactions dispatch only to nominal consumers; and
- legacy structural assertions dispatch only to legacy consumers.

The implementation MUST NOT take a nominal invocation, emit its payload as an
ordinary structural transient, and then run both new and old rules. Doing so
would preserve the cross-fire defect and could execute migrated and legacy
effects together.

Compatibility SHOULD be controlled by an explicit runtime capability/version,
not by guessing from argument shape. It may be removed only after the profile
and every active spot have completed migration, every installed command carries
a nominal identity artifact, no legacy command consumers remain installed, and
exported schemas contain no DOM-addressed command fields.

## Persisted-data migration

Changing bundled library YAML affects only repositories created after that
change. Existing profile and spot branches contain their own seeded command,
projection, view, and installed-rule assertions and MUST be migrated explicitly.

### Migration unit

Each branch migration is one reviewed notation document evaluated in one
transaction. It MUST contain, as applicable:

1. new nominal command declarations under stable identities;
2. event projection declarations;
3. replacement command-consuming rules;
4. explicit retractions of old installed rules;
5. replacement views only when their bindings must name a non-default
   projection; and
6. optional queries that make the expected affected entities visible in the
   evaluator response.

Repointing a command anchor is not enough. Existing rules are installed branch
data and continue to watch their old structural descriptors until retracted.

Old command descriptor facts need not be destructively deleted once no name,
view, rule, handler, or projection reaches them. They may be garbage-collected
separately.

### Rollback

The compatibility runtime remains installed throughout migration so a branch
can return to legacy dispatch. Each generated forward migration MUST be paired
with a rollback document that:

1. restores the prior command name targets and views where they changed;
2. reinstalls the exact legacy rules from their captured source;
3. retracts the replacement nominal rules; and
4. leaves the new, now-unreferenced command and projection artifacts in place.

The migration record MUST include the pre-migration revision, post-migration
revision, forward document, rollback document, dry-run output, and verification
result. Rollback is another reviewed notation transaction; it is not an
instruction to reset branch history or delete user data.

### Rule retraction

An old installed rule is retracted by its actual effect entity:

```yaml
rule!:
  this: effect:<existing-entity>
  ..: _
```

Migration tooling MUST discover these entities from the branch rather than
reconstructing their hashes. A generated migration MUST pair every replacement
rule with the legacy effect it supersedes, or explicitly state that no legacy
rule existed.

### Profile and spot scopes

The profile library is stored on the profile branch and is migrated once per
profile. It MUST NOT be copied into every spot merely because its commands are
used by the Hub.

Each spot branch is migrated independently. At the time of this specification
the locally active inventory is:

- `pi-harness-dev` — standard/core commands only;
- `recipe-tracker` — standard/core commands plus its custom add, remove,
  status, and quantity commands; and
- `tonk-team` — standard/core commands plus its note, vault, and workspace
  commands.

The inventory MUST be regenerated immediately before migration. The list above
is planning evidence, not an authorization to overwrite a branch whose schema
has since changed.

### Application procedure

Migration documents SHOULD be checked into the repository rather than existing
only as inspector history. For a spot, the CLI procedure is:

```bash
tonk eval migrations/recipe-tracker.notation \
  --spot recipe-tracker \
  --dry-run

tonk eval migrations/recipe-tracker.notation \
  --spot recipe-tracker
```

Pasting the same document into that spot's inspector is equivalent because the
inspector and CLI use the same evaluate pipeline. The profile migration must be
evaluated against the profile branch through the Hub/profile inspector or an
equivalent profile-scoped evaluate endpoint, not through an arbitrary spot.

Before applying a migration:

1. pull or otherwise confirm the branch revision to be migrated;
2. export its schema and installed command-consuming effects;
3. generate the migration against that exact revision;
4. review the command, projection, rule, and view diff;
5. dry-run the notation document; and
6. ensure the runtime version supports both old and new formats.

After applying it:

1. run the headless projection fixtures;
2. invoke each migrated command in the mounted browser UI;
3. verify the intended durable mutation;
4. reload and verify that the mutation persists;
5. verify that no unrelated command consumer ran;
6. verify the worker reports a selected consumer; and
7. export the schema again and confirm every migrated command is nominal and
   none retains a `dom.event/*` field.

Do not remove legacy runtime support until all of these checks pass for the
profile and every active spot.

## Bundled-library migration

All bundled `command!:` declarations must eventually be expressed as nominal
schemas. At the time of writing there are 47 declarations across
`core.yaml`, `profile.yaml`, `wiki.yaml`, `board.yaml`, `sheets.yaml`,
`table.yaml`, and `prose.yaml`.

For each declaration:

1. preserve or assign its stable command kind;
2. replace DOM-addressed fields with semantic attributes;
3. move extraction and browser actions into one or more projections;
4. update rule or handler registration to the command kind;
5. replace any domain-target use of transient `this` with a named argument;
6. remove marker fields that existed only to prevent structural cross-fire;
7. retain genuine domain arguments even if two commands share their shape; and
8. add a projection fixture and a dispatch-isolation test.

Updating bundled YAML supplies correct definitions to newly created profiles
and spots. It does not replace the persisted-data migration above.

## Rollout order

The implementation and rollout order is normative:

1. Add command identity and projection artifacts to analysis and schema output.
2. Add `invoke` validation and nominal dispatch to the worker and reactor.
3. Add projection resolution and typed extraction to the display runtime.
4. Add diagnostics and the headless projection verifier.
5. Add legacy/new dispatch isolation and compatibility tests.
6. Convert bundled libraries and their Rust handlers.
7. Generate and review the profile migration.
8. Migrate and browser-verify the profile.
9. Generate, apply, and browser-verify each active spot migration separately.
10. Confirm no active branch contains legacy DOM-addressed commands.
11. Remove legacy extraction and structural command dispatch in a later release.

The profile and spot migrations MUST NOT precede deployment of a compatible
runtime. Removal of compatibility MUST NOT be included in the same release as
the first migration.

## Canonical list-append benchmark

The acceptance benchmark is:

> Type text into a field, submit it, and observe one new list item.

The benchmark fixture MUST include:

- a durable list-item concept;
- a nominal add-item command with one required text argument;
- a submit projection using `control:` and `prevent-default`;
- a command-consuming rule that mints the durable item;
- a view containing the form and rendered list;
- a projection fixture for non-browser verification; and
- browser automation that types, submits, reloads, and observes persistence.

One run passes only if:

1. projection resolution succeeds;
2. the typed non-empty string appears unchanged in the `invoke` request;
3. exactly the add-item rule is selected;
4. exactly one durable item is added;
5. the page does not navigate or reload on submit;
6. the item remains after an explicit reload; and
7. no console or worker diagnostic has error severity.

Required negative cases are:

- the empty string reaches a text argument as `""` rather than disappearing;
- a missing named control fails with its projection and field names;
- a second command with the same schema does not fire;
- an unknown command argument is rejected;
- a missing required command argument is rejected;
- two projections without a unique default make a command-name binding fail
  as ambiguous; and
- nominal invocation does not trigger an installed legacy structural rule.

The benchmark task, fixture, structural verifier, browser verifier, and decision
rule MUST be versioned together.

## Acceptance criteria

The feature is complete only when:

- command identity is present on the wire and in installed rule/handler
  dispatch metadata;
- two identical command schemas remain dispatch-isolated;
- schema changes under one stable command kind do not change its identity;
- no new command declaration uses `the: dom.event/*`;
- event actions do not appear in rule premises or command payloads;
- required projection and invocation failures are visible and attributable;
- the CLI verifier exercises the production projection implementation;
- `tonk schema` round-trips commands and projections;
- legacy and nominal dispatch cannot both consume one invocation;
- the canonical list-append benchmark passes headlessly and in a mounted
  browser; and
- the profile and all active spots have reviewed, reversible migration evidence.

## Rejected alternatives

### Keep structural commands and only move DOM paths

Separating extraction would remove the action-field and path-resolution traps,
but commands with the same or overlapping schemas would still cross-fire and a
schema edit would still change matching behavior. This is a useful intermediate
refactor, not a sufficient endpoint.

### Add a unique marker argument to every command

A marker can simulate nominal dispatch in the structural engine, but it leaks
dispatch metadata into domain payloads, remains vulnerable to subset matching,
and requires every caller to manufacture a value whose only purpose is routing.
Existing marker fields should be removed during migration.

### Post the full descriptor plus a name hint

Treating the command name as an advisory check leaves the descriptor as the
real identity and preserves schema-coupled dispatch. The worker must select by
the command kind, not merely log it.

### Put browser actions on rule heads

The event object no longer exists when a worker rule fires, so actions such as
`preventDefault()` cannot be delayed until rule evaluation. They belong in the
main-thread projection and execute synchronously after successful extraction.

### Permit arbitrary JavaScript paths in V1 projections

That reproduces the current spelling, camel-casing, undefined-leaf, and
headless-fixture problems under a new declaration name. Typed sources cover the
known cases and give validation a finite contract.
