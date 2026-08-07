# Nominal commands and event projections implementation plan

**Goal:** Replace shape-selected event commands with stable nominal command identities and typed, headlessly testable event projections, while preserving a reversible compatibility path for every active profile and spot.

**Approach:** Introduce a distinct command input relation and `invoke` wire operation, then make rule and Rust-handler dispatch select consumers by command kind before decoding arguments. Store event projections as branch data, evaluate them through one source-independent engine shared by the browser and CLI, and retain the current structural path only as an explicitly isolated migration mode. Ship the compatible runtime before converting bundled libraries or applying reviewed forward/rollback notation documents to live branches.

**Constraints:**

- `docs/evolving-command-concepts.md` is normative; implementation choices in this plan must not weaken its invariants.
- Nominal invocations must never assert semantic argument attributes as ordinary transient facts, and must never reach legacy structural rules or handlers.
- The internal query representation may use reserved `dialog.command/*` EAVs, but those relations are private to selected nominal rules and must not resolve as ordinary concepts.
- `this` identifies the command occurrence. Domain targets are explicit arguments such as `todo`, `page`, or `space`.
- Required arguments reject absence, but a present empty text value remains `""`. Optional missing sources omit the argument.
- Declarative rules run inside the triggering transaction. Rule failure aborts it. Native handler registration is checked before commit, but handler work is scheduled only after commit and reports completion asynchronously by correlation ID.
- A registered rule whose durable premises do not match is a handled no-effect; a kind with no registered rule or native handler is an atomic error.
- Legacy and nominal formats coexist for migration, but one invocation is consumed by exactly one dispatch path.
- Persisted command declarations, rules, projections, and views are branch data. Updating bundled YAML does not migrate an existing profile or spot.
- The compatible runtime must be deployed before any profile or spot migration is applied. Compatibility removal is a later release and is not part of this plan.
- Preserve current `SourceClaim` assert/retract DAG-JSON, `TransactionBuilder::apply`, and `Commit::perform` callers through additive APIs where practical.
- Use existing workspace dependencies. Generate opaque invocation identifiers from `rand::random::<[u8; 16]>()`; do not add a UUID dependency.
- Keep native and wasm behavior aligned. Run focused native tests plus the repository's `test:web:debug` service-worker/browser suite for wasm-only dispatch and DOM behavior.

## Data and wire contracts

The implementation uses these storage relations. They are internal schema, but fixing the names here prevents the analyzer, resolver, CLI, and runtime from inventing incompatible shapes.

```text
(kind,   dialog.meta/command,       db:command)
(kind,   dialog.command/schema,     schema)
(schema, dialog.command/source,     <canonical CommandSchema JSON>)

(projection, dialog.meta/projection,    db:projection)
(projection, dialog.projection/command, kind)
(projection, dialog.projection/default, <bool>)
(projection, dialog.projection/source,  <canonical ProjectionDescriptor JSON>)

(effect, dialog.effect/command, kind)  # cardinality-many reverse index
```

`dialog.command/schema` is cardinality-one, so reasserting an anchored command updates its current schema without changing the kind. The schema and projection source strings are the exact bytes used for symmetric retraction, following the existing `Rule` pattern.

For query evaluation only, a command occurrence is encoded into the transaction overlay as:

```text
(occurrence, dialog.command/kind,             kind)
(occurrence, dialog.command.argument/<field>, value)
```

These attributes are never emitted under the command's semantic attribute identifiers and are never persisted. An analyzer-compiled command premise contains a constant `dialog.command/kind` term plus its argument terms. `dialog.effect/command` selects candidate rules first; the private descriptor then unifies arguments and joins durable premises in the existing query engine.

The public request remains exactly:

```json
{
  "claims": [
    {
      "op": "invoke",
      "command": "id:todo/add",
      "arguments": { "title": "Buy milk" }
    }
  ]
}
```

The successful response extends `TransactResponse` with:

```rust
pub struct InvocationOutcome {
    pub claim: usize,
    pub command: Entity,
    pub status: InvocationStatus, // Handled
    pub registered_rules: usize,
    pub fired_rules: usize,
    pub registered_handlers: usize,
    pub scheduled_handlers: usize,
    pub correlation: String,
}
```

Native completion is represented separately:

```rust
pub enum HandlerState { Scheduled, Completed, Failed }
pub struct HandlerOutcome {
    pub handler: String,
    pub state: HandlerState,
    pub message: Option<String>, // sanitized, never arguments
}
pub struct InvocationRecord {
    pub correlation: String,
    pub command: Entity,
    pub handlers: Vec<HandlerOutcome>,
}
```

`GET /api/invocations/{correlation}` returns the record. `TonkState` retains the newest 256 records in FIFO order; unknown or evicted identifiers return 404. This is diagnostic state, not durable branch data.

## File map

- `docs/evolving-command-concepts.md`: normative behavior and rollout contract.
- `plan/event-handling.md`: mark the structural `dom.event` design as compatibility-only and superseded.
- `rust/tonk-core/src/command.rs`: wire-neutral command schema, invocation, occurrence, batch, and private overlay encoding.
- `rust/tonk-core/src/claim.rs`: additive `SourceClaim::Invoke` wire variant.
- `rust/tonk-core/src/effect.rs`: discover command kinds referenced by compiled rules.
- `rust/tonk-schema/src/command_definition.rs`: stored command definition statement and branch resolver.
- `rust/tonk-schema/src/projection.rs`: projection descriptor, typed sources/actions, storage/resolution, and source-independent evaluator shared by CLI and display.
- `rust/tonk-schema/src/rule.rs`: persist and retract `dialog.effect/command` indexes exactly.
- `rust/tonk-schema/src/transact.rs`: analyzer IR and planner variants for command/projection declarations.
- `rust/tonk-analyzer/src/analyzer/{graph,declaration,scope,rule,error}.rs`: parse, resolve, validate, and compile nominal declarations and references.
- `rust/tonk-analyzer/src/{analyzer,analysis}.rs`: analysis output/lowering and behavioral tests.
- `rust/tonk-language-server/src/server.rs`: diagnostics regression tests for invalid projections.
- `rust/tonk-evaluator/src/{effect_query,effects}.rs`: nominal trigger validation, preflight lookup, fixpoint dispatch, and firing summary.
- `rust/dialog-reactor/src/{command,transaction}.rs`: typed nominal handler registry and additive commit reporting.
- `rust/tonk-worker/src/router/{command,transact}.rs`: preflight, scheduling, response outcomes, and completion recording.
- `rust/tonk-worker/src/router.rs`: invocation-status route and response export.
- `rust/tonk-worker/src/worker.rs`: bounded invocation ledger on `TonkState`.
- `rust/tonk-worker/src/router/{repository,join,session}.rs`: migrate custom native handlers to nominal arguments and structured outcomes.
- `rust/tonk-display/src/events/{dom,delegate}.rs`: browser adapter, projection resolution, and invoke posting.
- `rust/tonk-display/src/events/{extract,path}.rs`: retain only the legacy compatibility implementation.
- `rust/tonk-display/src/{events,element}.rs`: load/invalidate projection bindings and install delegates.
- `rust/tonk-cli/src/{project,commands,schema}.rs`: headless verifier, inventory, and schema rendering.
- `rust/tonk-cli/src/guide-events.md`: copy-runnable nominal event examples and diagnostics.
- `rust/tonk-cli/src/bin/tonk.rs`: `project` and `commands inventory` CLI surfaces.
- `rust/tonk-cli/tests/{project,commands,notation,schema_read}.rs`: CLI integration coverage.
- `rust/tonk-cli/Cargo.toml`: register the new integration tests.
- `rust/tonk-core/assets/library/{core,profile,wiki,board,sheets,table,prose}.yaml`: bundled nominal declarations and projections.
- `rust/tonk-worker/tests/{standard_library,fab_drift}.rs`: bundled schema and handler drift checks.
- `bench/scenarios/list-append/*`: frozen cold-build task, fixture, rubric, and structural verifier.
- `bench/README.md`: register and document the list-append scenario.
- `migrations/nominal-commands/*`: captured inventories plus forward and rollback notation for profile and active spots.

### Task 1: Add command wire and private input types

**Files:**

- Create: `rust/tonk-core/src/command.rs`
- Modify: `rust/tonk-core/src/lib.rs`
- Modify: `rust/tonk-core/src/claim.rs:SourceClaim, Claim, TryFrom<SourceClaim>`

**Interfaces:**

- Consumes: existing `ValueMap`, `Entity`, `Changes`, and assert/retract wire shapes.
- Produces: `CommandSchema`, `SourceInvocation`, `ValidatedInvocation`, `CommandValidationError`, `CommandOccurrence`, `CommandBatch`, `InvocationMetadata`, and `SourceClaim::Invoke(SourceInvocation)`.

- [ ] Add `source_claim_invoke_uses_command_specific_shape`, asserting the exact JSON above and confirming existing assert/retract fixtures are byte-for-byte unchanged.
- [ ] Add `command_batch_encodes_reserved_relations_without_semantic_attributes`: two identical invocations receive different occurrence entities, both carry `dialog.command/kind`, and neither contains `xyz.tonk.todo/title`.
- [ ] Add command validation tests for unknown, missing required, omitted optional, forbidden `this`, type mismatch, and a present empty text value that remains `Value::String("")`.
- [ ] Run `cargo test -p tonk-core command`; expect compile failure because the module and `Invoke` variant do not exist.
- [ ] Implement `CommandSchema { required: IndexMap<String, AttributeDescriptor>, optional: IndexMap<String, AttributeDescriptor> }`, `SourceInvocation { command: Entity, arguments: ValueMap }`, `CommandSchema::validate(SourceInvocation) -> Result<ValidatedInvocation, CommandValidationError>`, occurrence metadata, reserved attribute constructors, and batch encoding. Keep occurrence/correlation assignment outside `SourceInvocation`.
- [ ] Move the existing lossless `claim::cast` behavior behind a shared `coerce_value(field, expected, value)` helper and use it from both predicate and command validation so integer/entity/symbol coercion cannot drift.
- [ ] Implement custom or adjacently tagged serde so assert/retract retain `{ "op", "application" }` while invoke uses `{ "op", "command", "arguments" }`.
- [ ] Make `Claim::try_from` reject `Invoke` with a dedicated error explaining that authoritative branch resolution is required; the worker will route invokes before the predicate conversion path.
- [ ] Run `cargo test -p tonk-core command`; expect all command and claim tests to pass.
- [ ] Run `cargo test -p tonk-core`; expect success.

### Task 2: Persist and resolve nominal commands and projections

**Files:**

- Create: `rust/tonk-schema/src/command_definition.rs`
- Create: `rust/tonk-schema/src/projection.rs`
- Modify: `rust/tonk-schema/src/lib.rs`
- Modify: `rust/tonk-schema/src/transact.rs:Application, ApplicationPlan, Planner`

**Interfaces:**

- Consumes: Task 1 `CommandSchema`, existing `Source`, `QueryEnv`, `Statement`, `ThisIntent`, and name resolution.
- Produces: `CommandDefinition`, `CommandReference`, `ProjectionDescriptor`, `ProjectionSource`, `EventAction`, `ProjectionDefinition`, and analyzer/planner variants that emit definitions or carry a nominal invocation.

- [ ] Add `command_schema_replacement_preserves_kind`: assert `id:todo/add` with one schema, reassert with an added optional field, and verify one stable kind points to only the new content-derived schema entity.
- [ ] Add command assert/retract symmetry tests, including exact source bytes and removal of the old `dialog.command/schema` value.
- [ ] Add projection round-trip tests for every source form (`control.value`, `control.checked`, `data`, `event`, `detail`, `target`, `literal`) and all three actions.
- [ ] Add `projection_default_is_unique_per_command` resolver coverage with two default projections returning `ProjectionResolveError::AmbiguousDefault`.
- [ ] Run `cargo test -p tonk-schema command_definition` and `cargo test -p tonk-schema projection`; expect missing-module failures.
- [ ] Implement the storage adapters exactly as specified in “Data and wire contracts”. Canonicalize JSON with the repository's DAG-JSON/serde path before hashing or storing it.
- [ ] Implement branch lookup by anchor/name and entity, including current-schema resolution, projections-for-command, and exact stored-source retraction builders.
- [ ] Add distinct `Application::{CommandDefinition, ProjectionDefinition, CommandInvocation}` and matching plan variants. Definitions may assert/retract, command invocation may only assert/invoke, and ordinary queries cannot treat any of them as a structural concept.
- [ ] Run `cargo test -p tonk-schema command_definition` and `cargo test -p tonk-schema projection`; expect success.
- [ ] Run `cargo test -p tonk-schema`; expect success.

### Task 3: Analyze `command!:` as nominal and add `projection!:`

**Files:**

- Modify: `rust/tonk-analyzer/src/analyzer/graph.rs:Need, DeclarationKind, Graph::push, Graph::resolve`
- Modify: `rust/tonk-analyzer/src/analyzer/declaration.rs`
- Modify: `rust/tonk-analyzer/src/analyzer/scope.rs`
- Modify: `rust/tonk-analyzer/src/analyzer/error.rs`
- Modify: `rust/tonk-analyzer/src/analysis.rs`
- Test: `rust/tonk-analyzer/src/analyzer.rs`
- Test: `rust/tonk-language-server/src/server.rs`

**Interfaces:**

- Consumes: Task 2 resolvers and plan variants; generic `tonk-notation` mapping syntax.
- Produces: stable command kinds, validated `ProjectionDefinition`s, nominal standalone invocations, and source-ranged diagnostics.

- [ ] Add analyzer tests proving: anchored `command!: &todo/add` gets kind `id:todo/add`; explicit `this:` wins while the anchor aliases it; and an unanchored/no-`this` command is rejected.
- [ ] Add a replacement test proving an edited optional schema changes the schema entity but not the command kind.
- [ ] Add projection tests for unknown command, unknown argument, missing required source, unsupported event member, unsupported action, unanchored projection, and duplicate default. Assert the diagnostic range covers the offending key/value.
- [ ] Add one whole-document test containing attribute, command, projection, rule, and view declarations and assert the lowered plans in document order.
- [ ] Add standalone application coverage: `todo/add!:\n  title: Buy milk` lowers to `SourceClaim::Invoke { command: id:todo/add, arguments: { title: Buy milk } }`; command retraction and ordinary top-level command query forms are rejected.
- [ ] Run `cargo test -p tonk-analyzer nominal_command`; expect failures because `command!:` still lowers as transient `concept!:`.
- [ ] Extend graph needs and scope tables with command/projection references. Parse command `with:` as required and `maybe:` as optional without setting the structural transient marker.
- [ ] Implement the projection parser with the exact source/action whitelist from the spec; reject multiple source keys in one argument mapping.
- [ ] Enforce default uniqueness against both in-document and branch-resolved projections.
- [ ] Expose command/projection applications through `Analysis` and lower them through Task 2 planner variants.
- [ ] Run `cargo test -p tonk-analyzer`; expect success.
- [ ] Add LSP regression assertions for the unknown control source form and duplicate default errors, then run `cargo test -p tonk-language-server`; expect success.

### Task 4: Compile and index nominal command rules

**Files:**

- Modify: `rust/tonk-analyzer/src/analyzer/rule.rs`
- Modify: `rust/tonk-core/src/effect.rs:Effect`
- Modify: `rust/tonk-schema/src/rule.rs:Statement for Rule`
- Modify: `rust/tonk-evaluator/src/effect_query.rs`
- Test: `rust/tonk-analyzer/src/analyzer/rule.rs`
- Test: `rust/tonk-schema/src/rule.rs`

**Interfaces:**

- Consumes: resolved command kind/schema from Task 3 and the private relation names from Task 1.
- Produces: command-premise/head descriptors, `Effect::command_kinds()`, and exact `dialog.effect/command` indexes.

- [ ] Add a compiler test for `when: [{ assert: todo/add, where: { title: ?title } }]`; assert the descriptor contains constant kind `id:todo/add`, reserved occurrence `this`, and reserved argument relation—not `xyz.tonk.todo/title`.
- [ ] Add a command-head test proving a rule may emit `todo/add` for the next round and cannot assign `this` as an argument.
- [ ] Add isolation tests: two commands with the same `title` schema produce different `dialog.effect/command` indexes; a durable concept with the same field shape produces none.
- [ ] Add assert/retract symmetry coverage for command indexes using a stable `effect:` entity and exact stored source.
- [ ] Change install validation tests so a positive nominal command premise satisfies the trigger requirement, while a durable-only rule still fails.
- [ ] Run `cargo test -p tonk-analyzer command_rule`, `cargo test -p tonk-schema command_index`, and `cargo test -p tonk-evaluator nominal_trigger`; expect failures on structural compilation/indexing.
- [ ] Compile a private command predicate with `dialog.command/kind = <kind>` and `dialog.command.argument/<field>` terms. Keep other premises unchanged so the query engine joins command variables to durable concepts normally.
- [ ] Implement `Effect::command_kinds()` by recognizing only the analyzer's private descriptor marker; exclude those relations from `on_entities()`.
- [ ] Emit/dissociate one `dialog.effect/command` claim per kind. Extend effect loading/preflight queries without altering legacy `dialog.effect/on` behavior.
- [ ] Run the focused tests and `cargo test -p tonk-core -p tonk-schema -p tonk-analyzer -p tonk-evaluator`; expect success.

### Task 5: Evaluate command occurrences transactionally

**Files:**

- Modify: `rust/tonk-evaluator/src/effects.rs:TransactionExt, Induce, fixpoint loop`
- Modify: `rust/dialog-reactor/src/transaction.rs:TransactionBuilder, Commit`
- Test: `rust/tonk-evaluator/src/effects.rs`
- Test: `rust/dialog-reactor/src/transaction.rs`

**Interfaces:**

- Consumes: `CommandBatch`, `dialog.effect/command`, and compiled command heads.
- Produces: `InduceSummary { registered_rules_by_occurrence, fired_rules_by_occurrence }`, `CommitReport { revision, induction }`, `Commit::perform_report`, and preserved `Commit::perform -> Revision`.

- [ ] Add a fixpoint test where `todo/add` plus durable collection state appends one durable todo and leaves no `dialog.command/*` facts after commit.
- [ ] Add repeated-occurrence coverage: two identical invoke claims append twice and have independent firing counts.
- [ ] Add a command-to-command two-round test and a cycle test that still fails at `MAX_ROUNDS` and commits nothing.
- [ ] Add the critical isolation test: invoking command A does not fire command B's same-shaped nominal rule or a legacy structural rule.
- [ ] Add registered-but-no-match coverage: preflight count is one, fired count is zero, and the commit succeeds.
- [ ] Run `cargo test -p tonk-evaluator nominal`; expect failures because induction only accepts structural transient `Changes`.
- [ ] Extend the fixpoint with separate `CommandBatch` stimulus. Encode each round into reserved overlay changes, select effects exclusively through `dialog.effect/command`, freeze sibling-rule input as today, parse command heads into next-round occurrences, and sweep every reserved fact before commit.
- [ ] Validate rule-emitted command arguments against the command schema before promoting them to the next round; return a rule-attributed `InduceError` on invalid output.
- [ ] Add `perform_report` APIs while leaving existing `perform` call sites source-compatible by discarding the report.
- [ ] Run `cargo test -p tonk-evaluator nominal` and `cargo test -p dialog-reactor command_occurrence`; expect success.
- [ ] Run `cargo test -p tonk-evaluator -p dialog-reactor`; expect success.

### Task 6: Select and decode native handlers by command kind

**Files:**

- Modify: `rust/dialog-reactor/src/command.rs`
- Modify: `rust/dialog-reactor/src/lib.rs`
- Test: `rust/dialog-reactor/src/command.rs`

**Interfaces:**

- Consumes: Task 1 occurrences and existing `Concept`-derived Rust command structs.
- Produces: nominal registration/decoding while retaining a clearly named legacy registry path.

```rust
pub trait CommandHandler<Env> {
    fn kind(&self) -> &Entity;
    fn name(&self) -> &'static str;
    fn decode(&self, occurrence: &CommandOccurrence) -> Option<BoxedCommandRun<Env>>;
}

#[cfg(not(target_arch = "wasm32"))]
pub type RunFuture = Pin<Box<dyn Future<Output = Result<(), CommandFailure>> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
pub type RunFuture = Pin<Box<dyn Future<Output = Result<(), CommandFailure>> + 'static>>;

impl<Env> CommandRegistry<Env> {
    pub fn nominal<C>(self, kind: Entity) -> Self;
    pub fn register_nominal(&mut self, handler: Box<dyn CommandHandler<Env>>);
    pub fn registrations(&self, kind: &Entity) -> usize;
    pub fn schedule(&self, occurrence: &CommandOccurrence, env: &Env) -> Vec<ScheduledHandler>;
    pub fn register_legacy(&mut self, handler: Box<dyn LegacyCommandHandler<Env>>);
}
```

- [ ] Add tests registering command A and B with identical Rust field descriptors; only the handler registered under the invoked kind may decode/run.
- [ ] Add optional-field decode coverage using `Option<T>`, empty-string decode coverage, missing-required rejection, and occurrence `this` binding.
- [ ] Add a compatibility test proving `match_legacy_transients` cannot see a `CommandBatch` and `schedule` cannot see legacy `Changes`.
- [ ] Add a failure test proving a handler future returns `CommandFailure { code, message }` instead of swallowing/logging the error.
- [ ] Run `cargo test -p dialog-reactor command::tests`; expect failures on attribute-indexed matching.
- [ ] Build typed nominal decode by binding the Rust concept application's field variables directly from `occurrence.arguments` and binding `this` from the occurrence. Do not synthesize semantic `EntityFacts`.
- [ ] Split or rename the existing attribute-indexed code as the legacy compatibility path; remove ambiguous `.register()`/`.command()` names from new call sites.
- [ ] Run `cargo test -p dialog-reactor command::tests`; expect success.
- [ ] Run `cargo test -p dialog-reactor`; expect success.

### Task 7: Make `/transact` preflight, commit, and schedule explicit

**Files:**

- Modify: `rust/tonk-worker/src/router/transact.rs`
- Modify: `rust/tonk-worker/src/router/command.rs`
- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-worker/src/worker.rs`
- Test: `rust/tonk-worker/src/router/transact.rs`
- Test: `rust/tonk-worker/src/router/command.rs`

**Interfaces:**

- Consumes: Task 2 authoritative schema resolver, Task 5 `perform_report`, and Task 6 registry.
- Produces: atomic preflight, `TransactResponse.invocations`, post-commit scheduling, bounded status lookup, and structured errors.

- [ ] Add route tests for unknown kind, unknown argument, missing required argument, forbidden `this`, and no registered consumer. Assert no revision/tree change and stable codes `command_unknown`, `command_argument_unknown`, `command_argument_missing`, `command_argument_reserved`, and `command_unhandled`.
- [ ] Add a mixed-batch test where one valid durable assertion plus one invalid invocation commits neither.
- [ ] Add success tests for: one fired rule; one registered rule with zero matches; one scheduled native handler; and both a rule and handler on the same kind.
- [ ] Add response coverage for claim index, kind, all four counts, and a nonempty correlation that is absent from command arguments.
- [ ] Add asynchronous ledger tests: scheduled before run, completed after `Ok`, failed with sanitized detail after `Err`, sibling failures independent, FIFO eviction at 257 records, and 404 for unknown IDs.
- [ ] Run `cargo test -p tonk-worker transact::nominal`; expect failures because `Invoke` reaches `Claim::try_from` or lacks branch validation.
- [ ] Parse all claims first. Resolve and validate every invoke against the target branch; assign `invoke:<32 lowercase hex chars>` using `rand::random::<[u8; 16]>()`; query registered rule counts; query native registration counts; and abort before building/committing if any invocation is invalid or unhandled.
- [ ] Feed validated occurrences into `TransactionBuilder`, call `perform_report`, and build `InvocationOutcome` from preflight plus induction counts.
- [ ] Only after a successful commit, create ledger records and spawn the native futures. Each future updates only its `(correlation, handler)` slot, then the existing dispatcher drains scheduled polls.
- [ ] Keep legacy transient dispatch after commit, but pass it only the legacy `Changes` snapshot. Never convert nominal occurrences into that snapshot.
- [ ] Register `GET /api/invocations/{correlation}` and return diagnostic records without payload values.
- [ ] Run `cargo test -p tonk-worker transact::nominal`; expect success.
- [ ] Run `cargo test -p tonk-worker`; expect success on native tests.

### Task 8: Build one source-independent projection evaluator

**Files:**

- Modify: `rust/tonk-schema/src/projection.rs`
- Test: `rust/tonk-schema/src/projection.rs`

**Interfaces:**

- Consumes: Task 2 `ProjectionDescriptor` and Task 1 `CommandSchema::validate`/`SourceInvocation`.
- Produces: `ProjectionInput`, `ProjectionResult`, `ProjectionTrace`, and `ProjectionError`, usable without `web_sys`.

```rust
pub trait ProjectionInput {
    fn control(&self, name: &str, property: ControlProperty) -> SourceRead;
    fn data(&self, name: &str) -> SourceRead;
    fn event(&self, member: EventMember) -> SourceRead;
    fn detail(&self, member: &str) -> SourceRead;
    fn target(&self, member: TargetMember) -> SourceRead;
}

pub fn project(
    projection: &ProjectionDefinition,
    schema: &CommandSchema,
    input: &impl ProjectionInput,
) -> Result<ProjectionResult, ProjectionError>;
```

- [ ] Add fixture-adapter tests for every source form and command value type already supported by typed applications.
- [ ] Add exact-name coverage for `name="note-body"` and `data-note-id`; no camel-casing is allowed.
- [ ] Add blank handling coverage: required text `""` is present; optional missing is omitted and traced; required missing aborts the whole projection; `false` and numeric zero remain present.
- [ ] Add ordering coverage proving no actions are returned when extraction/coercion fails and successful actions preserve declaration order.
- [ ] Run `cargo test -p tonk-schema projection_evaluator`; expect failures because the storage module has no evaluator.
- [ ] Implement `ProjectionInput` with explicit `Present(Value)`, `Missing`, and `ReadFailed` outcomes so missing and failed reads cannot collapse together.
- [ ] Reuse the command schema's existing cast/coercion rules instead of implementing browser-specific conversions.
- [ ] Return a complete `SourceInvocation`, per-field trace, omitted optional list, and planned actions; never execute actions in the shared core.
- [ ] Run `cargo test -p tonk-schema projection_evaluator`; expect success.

### Task 9: Use projections in the mounted browser runtime

**Files:**

- Create: `rust/tonk-display/src/events/dom.rs`
- Modify: `rust/tonk-display/src/events/delegate.rs`
- Modify: `rust/tonk-display/src/events/extract.rs`
- Modify: `rust/tonk-display/src/events/path.rs`
- Modify: `rust/tonk-display/src/element.rs:install_event_delegate`
- Test: `rust/tonk-display/src/events/{dom,delegate}.rs`
- Test: `rust/tonk-display/src/view.rs`

**Interfaces:**

- Consumes: Task 8 evaluator from `tonk-schema` and existing `preprocess` `data-on<event>` bindings.
- Produces: projection resolver cache, synchronous DOM actions, nominal invoke request, structured console diagnostics, and isolated legacy fallback.

- [ ] Add wasm tests for exact `form.elements.namedItem` lookup from both a bound form and a button's `.form`, including `name="note-body"`.
- [ ] Add resolution-order tests: explicit projection name; command with one projection; unique default among several; ambiguous command reference; unresolved binding; legacy fallback only when the reference resolves to a legacy command with no projection.
- [ ] Add a submit test proving `preventDefault` runs synchronously after successful extraction and the emitted claim is `op: invoke` with an empty text value retained.
- [ ] Add failure tests proving missing required controls, coercion errors, and ambiguous projection emit no request and name the projection, command, field, and source in the diagnostic.
- [ ] Add cache invalidation coverage: changing command/projection/name subscription data rebuilds the resolver before the next event without remounting the whole page.
- [ ] Build the web test archive with `nix build .#tests-web-debug`, then run `cargo nextest run --workspace-remap ./ --archive-file "$(nix eval .#tests-web-debug.outPath --raw)/tests-web-debug.tar.zst" 'tonk_display::events'`; expect failures because the delegate resolves only concept descriptors and posts asserts.
- [ ] Implement the live DOM adapter with the finite member whitelist. Execute `preventDefault`, `stopPropagation`, and `stopImmediatePropagation` synchronously in the returned order, then post the invoke through the existing host claim bridge.
- [ ] Replace `Descriptors` with `BindingsCatalog { commands, projections, legacy_descriptors }`. Fetch its rows during delegate installation and refresh it on the existing phase-1 subscription revision path.
- [ ] Move current `build_transact_body` and camel-cased path walking behind a `legacy` module/API and call it only from the explicit compatibility branch.
- [ ] Log structured diagnostics as one JSON object; never include argument values by default.
- [ ] Rebuild with `nix build .#tests-web-debug`, then run `cargo nextest run --workspace-remap ./ --archive-file "$(nix eval .#tests-web-debug.outPath --raw)/tests-web-debug.tar.zst" 'tonk_display::events'`; expect success.
- [ ] Run `test:web:debug`; expect the full wasm archive to pass.

### Task 10: Add schema rendering, inventory, and headless verification to the CLI

**Files:**

- Create: `rust/tonk-cli/src/project.rs`
- Create: `rust/tonk-cli/src/commands.rs`
- Modify: `rust/tonk-cli/src/lib.rs`
- Modify: `rust/tonk-cli/src/bin/tonk.rs`
- Modify: `rust/tonk-cli/src/schema.rs`
- Create: `rust/tonk-cli/tests/project.rs`
- Create: `rust/tonk-cli/tests/commands.rs`
- Modify: `rust/tonk-cli/tests/schema_read.rs`
- Modify: `rust/tonk-cli/tests/notation.rs`
- Modify: `rust/tonk-cli/Cargo.toml`

**Interfaces:**

- Consumes: Task 2 resolution and Task 8 evaluator from `tonk-schema` through a non-wasm `FixtureInput` implementation.
- Produces: `tonk project`, `tonk commands inventory --json`, and re-submittable command/projection schema notation.

- [ ] Add `tonk schema` round-trip coverage: export a nominal command and projection, evaluate the export into an empty spot, and compare stable kind, schema, projection, and default flag.
- [ ] Define fixture YAML as maps named `controls`, `data`, `event`, `detail`, and `target`; controls contain `{ value, checked }`. Add tests for hyphenated names, blank required text, optional omission, and each error classification.
- [ ] Add CLI output snapshots containing resolved projection/kind, field source traces, omissions, planned actions, and exact invoke JSON. Verify `--redact` replaces values but not field/source names.
- [ ] Add `--transact` coverage against a disposable spot with one declarative rule, and assert the durable result. Default invocation must leave the revision unchanged.
- [ ] Add inventory JSON coverage listing nominal/legacy declarations, projection references, command-consuming effect entities/source, event bindings, and observed branch revision.
- [ ] Run `cargo test -p tonk-cli --test project --test commands --test schema_read --test notation`; expect missing-command and round-trip failures.
- [ ] Add CLI syntax:

  ```text
  tonk project <PROJECTION_OR_COMMAND> --fixture <PATH> [--json] [--redact] [--transact]
  tonk commands inventory --json
  ```

  Both obey `--spot > TONK_SPOT > directory binding`; `project` is non-mutating unless `--transact` is explicit.
- [ ] Have `--transact` submit the produced `SourceInvocation` through the same local `TransactionBuilder` validation/preflight path, with declarative rules only; an empty local native registry must not masquerade as a consumer.
- [ ] Render nominal command kinds with `command!:` and projections with `projection!:`. Keep legacy commands renderable during compatibility and label them only in JSON inventory, not with invalid notation.
- [ ] Run the focused CLI command; expect success.
- [ ] Run `cargo test -p tonk-cli`; expect success.

### Task 11: Migrate bundled command schemas and native handlers

**Files:**

- Modify: `rust/tonk-core/assets/library/core.yaml`
- Modify: `rust/tonk-core/assets/library/profile.yaml`
- Modify: `rust/tonk-core/assets/library/wiki.yaml`
- Modify: `rust/tonk-core/assets/library/board.yaml`
- Modify: `rust/tonk-core/assets/library/sheets.yaml`
- Modify: `rust/tonk-core/assets/library/table.yaml`
- Modify: `rust/tonk-core/assets/library/prose.yaml`
- Modify: `rust/tonk-schema/src/command.rs`
- Modify: `rust/tonk-worker/src/router/command.rs`
- Modify: `rust/tonk-worker/src/router/repository.rs`
- Modify: `rust/tonk-worker/src/router/join.rs`
- Modify: `rust/tonk-worker/src/router/session.rs`
- Test: `rust/tonk-worker/tests/standard_library.rs`
- Test: `rust/tonk-worker/tests/fab_drift.rs`

**Interfaces:**

- Consumes: Tasks 3–9 nominal rule/handler/browser path.
- Produces: 47 bundled nominal declarations, their projections, and kind-registered Rust handlers with typed optional fields.

- [ ] Extend the standard-library audit to fail if any new command schema contains `dom.event/*`, `dom.event.do/*`, or a marker-only argument, if any event binding lacks an unambiguous projection, or if the declaration count differs from the captured inventory.
- [ ] Add per-library tests that commands with overlapping schemas dispatch only their own rule. Include at least wiki create/rename, board create/edit, table create/edit, and workspace activate/close pairs.
- [ ] Add Rust-handler registration tests for all nine current custom handlers: `CreateSpace`, `RemoveSpace`, `Invite`, `EnableSync`, `PauseSync`, `ProfileRename`, `RenameRepository`, `Join`, and `Load`.
- [ ] Run `cargo test -p tonk-worker --test standard_library --test fab_drift`; expect failures listing all structural/DOM-addressed commands.
- [ ] Convert declarations one library at a time. Keep each current anchor as the nominal kind; add semantic attributes; add one or more projections; update rule premises/heads; replace target use of `this` with explicit names.
- [ ] Specifically migrate `CreateSpace.remote` and `Invite.space` to declared optional typed arguments. Nominal handlers must decode the typed fields. Keep the existing raw-fact readers only inside separately registered legacy adapters until the later compatibility-removal release.
- [ ] Remove timestamp/marker arguments used only for uniqueness. Preserve timestamps only where downstream behavior genuinely consumes them.
- [ ] Change `Load` to declare an explicit `site` argument; bind the occurrence separately as `this`; update its producer and handler accordingly.
- [ ] Register nominal native handlers by their stable kind and keep distinct legacy registrations for unmigrated branches; both may call shared operational functions, but their decoders and routing indexes must remain separate. Convert swallowed failures into `Result<(), CommandFailure>`; successful no-op policy decisions must be explicit `Ok(())`, while operational failures must become ledger failures.
- [ ] After each library conversion, run its focused standard-library filter and `cargo test -p tonk-analyzer -p tonk-schema -p tonk-evaluator`.
- [ ] Run `cargo test -p tonk-worker --test standard_library --test fab_drift`; expect success.
- [ ] Run `test:web:debug`; expect mounted Hub/FAB/site command flows to pass.

### Task 12: Replace the event guide and freeze the list-append benchmark

**Files:**

- Modify: `rust/tonk-cli/src/guide-events.md`
- Modify: `plan/event-handling.md`
- Modify: `rust/tonk-core/docs/templates.md`
- Modify: `bench/README.md`
- Create: `bench/scenarios/list-append/task.md`
- Create: `bench/scenarios/list-append/rubric.md`
- Create: `bench/scenarios/list-append/scenario.env`
- Create: `bench/scenarios/list-append/prepare.sh`
- Create: `bench/scenarios/list-append/scripted.sh`
- Create: `bench/scenarios/list-append/checkpoints`
- Create: `bench/scenarios/list-append/fixture.yaml`
- Create: `bench/scenarios/list-append/verify.sh`

**Interfaces:**

- Consumes: final notation and `tonk project` syntax.
- Produces: a copy-runnable example and frozen first-shot pass/fail benchmark.

- [ ] Replace the structural create-form advice with one complete block containing attribute, durable todo/list membership concepts, nominal command, stable rule entity, submit projection, view, and fixture.
- [ ] The canonical markup must be a normal `<form onsubmit=todo/add>` with `<input name="title">` and `prevent-default` in the projection; remove the old advice that requires `type="button"` and `current-target.form.elements.*`.
- [ ] Include diagnostics examples for missing control, missing argument, ambiguous projection, unhandled command, registered rule/no durable match, and failed native completion.
- [ ] Mark `plan/event-handling.md` “Superseded for new code”; preserve its text as the legacy compatibility contract until removal.
- [ ] Make `prepare.sh` create a blank disposable spot and no application code. The task prompt is exactly: “Build a todo list: type text into a field, submit it, and append the text to a persistent list.”
- [ ] Make `scripted.sh` install the canonical notation and mounted view, providing the harness's known-good reference path without changing the cold agent task.
- [ ] Make `verify.sh` query durable todos after the browser interaction and require exactly the submitted nonempty title once; separately run `"$TONK" project todo/add-form --spot "$TONK_SPOT" --fixture "$SCENARIO/fixture.yaml" --json` and require the projected invoke to carry the fixture's title.
- [ ] Define pass as all of: cold agent completes without issue-specific hints; structural verifier passes; mounted browser submission appends; reload retains the item; console has no projection/invocation error; unrelated same-shaped command consumer does not fire.
- [ ] Add `list-append` to the scenario table in `bench/README.md`, including its exact first-shot decision rule.
- [ ] Run `nix develop -c bench/bin/bench run list-append --scripted`; expect the harness to discover the scenario and write `verify.json` with `passed: true`.

### Task 13: Add end-to-end compatibility and failure-surface tests

**Files:**

- Modify: `rust/tonk-worker/src/router.rs`
- Modify: `rust/tonk-worker/src/router/transact.rs`
- Modify: `rust/tonk-display/src/element.rs`
- Modify: `rust/tonk-cli/tests/project.rs`
- Test: existing wasm route/display test modules in those files

**Interfaces:**

- Consumes: all runtime work above.
- Produces: release-gating evidence for the compatibility deployment.

- [ ] Add a mounted-browser E2E fixture with two same-shaped commands and one list-append rule. Submit through a real DOM event and assert only the selected durable list changes.
- [ ] Add the four original silent failures as explicit tests: undefined source, blank required value handling, action present on projection rather than payload, and same-shaped consumer isolation. Each must produce either a successful durable mutation or a stable visible diagnostic—never a silent no-op.
- [ ] Add a legacy fixture proving an unmigrated `onclick=<legacy-command>` still posts an assert and fires its legacy consumer while compatibility is enabled.
- [ ] In the same fixture, prove a nominal invoke cannot fire that legacy consumer and a legacy assert cannot fire the nominal consumer.
- [ ] Add a response/ledger browser test: native handler is reported `scheduled` in `/transact`, later becomes `completed` or `failed` through `/api/invocations/{correlation}`, and the triggering revision remains committed on failure.
- [ ] Run `cargo test -p tonk-worker -p tonk-cli`; expect success.
- [ ] Run `test:web:debug`; expect success, including the mounted list-append and compatibility filters.
- [ ] Run `cargo fmt --check`; expect no diff.

### Task 14: Generate reversible migrations without applying them

**Files:**

- Create: `migrations/nominal-commands/README.md`
- Create: `migrations/nominal-commands/profile.inventory.json`
- Create: `migrations/nominal-commands/profile.forward.notation`
- Create: `migrations/nominal-commands/profile.rollback.notation`
- Create: `migrations/nominal-commands/pi-harness-dev.inventory.json`
- Create: `migrations/nominal-commands/pi-harness-dev.forward.notation`
- Create: `migrations/nominal-commands/pi-harness-dev.rollback.notation`
- Create: `migrations/nominal-commands/recipe-tracker.inventory.json`
- Create: `migrations/nominal-commands/recipe-tracker.forward.notation`
- Create: `migrations/nominal-commands/recipe-tracker.rollback.notation`
- Create: `migrations/nominal-commands/tonk-team.inventory.json`
- Create: `migrations/nominal-commands/tonk-team.forward.notation`
- Create: `migrations/nominal-commands/tonk-team.rollback.notation`

**Interfaces:**

- Consumes: the deployed-compatible schema format and Task 10 inventory output.
- Produces: reviewed, revision-pinned forward/rollback artifacts. This task does not mutate profile or spot state.

- [ ] Immediately before generation, run `tonk pull --spot <spot>` and `tonk commands inventory --spot <spot> --json` for `pi-harness-dev`, `recipe-tracker`, and `tonk-team`; capture exact revisions. Capture the profile through the profile-scoped inspector/evaluate API, not an arbitrary spot.
- [ ] For every legacy rule, copy its actual effect entity and exact stored source into the inventory. Pair it with either one replacement rule or an explicit “no replacement” reason.
- [ ] Generate each forward document with nominal declarations, projections, replacement rules, old-rule retractions by actual `effect:` entity, and view replacements only where the binding cannot use a unique default.
- [ ] Generate each rollback document with the old name/view targets and exact legacy rules, replacement-rule retractions, and no deletion of new unreferenced schema/projection artifacts.
- [ ] Run each spot forward document with `tonk eval <file> --spot <spot> --dry-run`; expect successful analysis and a diff limited to the inventoried command/projection/rule/view artifacts.
- [ ] Run each rollback document against a disposable clone or temporary branch containing the dry-run forward result; expect the exported legacy command/rule/view inventory to match the pre-migration inventory.
- [ ] Record pre-revision, `post_revision: null` (filled only on application), runtime compatibility version, dry-run output hash, reviewer, and test result in `README.md`.
- [ ] Stop here. Do not paste into inspectors or run non-dry-run `tonk eval` until Task 13 is green in the deployed runtime and the individual branch application is explicitly approved.

### Task 15: Apply and verify migrations one branch at a time

**Files:**

- Modify after each approved application: `migrations/nominal-commands/README.md`

**Interfaces:**

- Consumes: compatible deployed runtime and reviewed Task 14 documents.
- Produces: per-branch application evidence. Each branch is an independent release operation and rollback decision.

- [ ] Confirm the live revision still equals the inventory revision. If it differs, regenerate both forward and rollback files; do not apply a stale migration.
- [ ] Apply the profile forward document once through the Hub/profile inspector or equivalent profile-scoped evaluate endpoint. Record the resulting revision.
- [ ] Run its projection fixtures, exercise every migrated Hub/FAB native command in the mounted browser, poll native completion records, reload, and export inventory. On failure, apply the reviewed profile rollback document and record both revisions.
- [ ] Repeat independently for `pi-harness-dev`, then `recipe-tracker`, then `tonk-team` with `tonk eval <forward> --spot <spot>`. Do not batch branches into one approval or one evidence record.
- [ ] For each spot, verify intended durable changes, reload persistence, zero unrelated consumer fires, no `dom.event/*` command fields, no remaining legacy command-consuming effects, and schema/projection round-trip.
- [ ] Update `README.md` with actual before/after revisions, forward/rollback document hashes, headless result, mounted-browser result, and whether rollback was exercised.
- [ ] Run `tonk commands inventory --json` again for profile and all three spots. Compatibility removal is eligible for a separate plan only when all inventories report zero legacy consumers and all evidence is complete.

## Final verification gate

- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test -p tonk-core -p tonk-schema -p tonk-analyzer -p tonk-evaluator -p dialog-reactor -p tonk-worker -p tonk-cli`.
- [ ] Run `cargo test -p tonk-worker --test standard_library --test fab_drift`.
- [ ] Run `cargo test -p tonk-cli --test notation --test schema_read --test project --test commands`.
- [ ] Run `test:web:debug` and preserve the focused list-append, dispatch-isolation, and handler-ledger output.
- [ ] Run the frozen list-append reference scenario headlessly and in a mounted browser; preserve its structural JSON and browser evidence.
- [ ] Run `nix flake check 'path:.'`; if an unrelated aggregate wasm timeout occurs, report it explicitly and preserve the successful focused filters rather than claiming the aggregate passed.
- [ ] Review `git diff --check`, `git status --short`, the spec-to-task requirement map, and all migration documents before calling the compatible runtime ready.

## Requirement map

- Stable nominal identity and schema evolution: Tasks 1–4.
- Rule identity, occurrence semantics, command chains, and atomic rule failure: Tasks 4–5.
- Nominal native selection, post-commit scheduling, and asynchronous completion: Tasks 6–7.
- Typed projection language, exact DOM semantics, actions, and cache invalidation: Tasks 8–9.
- Headless production-path verification and schema introspection: Task 10.
- Bundled commands and Rust handlers: Task 11.
- Worked example and first-shot benchmark: Task 12.
- Loud failure modes and dispatch isolation: Task 13.
- Profile and active-spot forward/rollback migration: Tasks 14–15.
- Later removal of legacy dispatch: intentionally excluded until Task 15 evidence is complete.
