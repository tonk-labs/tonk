---
name: target-agnostic-commands
description: Commands are intent; a Provider fulfills them per runtime. Define both target-agnostically so the same command runs in the browser, the CLI, and tests. Use whenever adding or changing a command handler, writing `impl CommandHandler`, adding `#[cfg(target_arch = "wasm32")]` to worker code, or reaching for raw `EntityFacts` instead of a declared command field.
allowed-tools: Read, Bash, Glob, Grep
---

# Target-agnostic commands

A command describes **intent**. A `Provider<C>` **fulfills** it. The
runtime differences belong inside the provider, not in the command and
not in whether the command exists at all.

```rust
impl Provider<CreateSpace> for Web    { /* passkey ceremony, postMessage navigate */ }
impl Provider<CreateSpace> for Native { /* fs bookkeeping, re-point the mounted space */ }
```

Both delegate to the shared env for the universal part. That is the whole
point of the trait: `Provider<C>` is implemented **on an environment**, and
`CommandRegistry<Env>` is generic over one. A command written this way runs
in the service worker, in the CLI, and in a native test without changing.

## The rule

**Never weld a command to one runtime.** Concretely, do not:

- write `impl CommandHandler<CommandEnv>` by hand when a `Provider<C>` will do;
- gate a handler `#[cfg(target_arch = "wasm32")]` because its neighbour is gated;
- read a value out of raw `EntityFacts` instead of declaring it as a command field;
- register commands only in the wasm branch of `command_registry()`.

**If it feels like you need a target-specific escape hatch, stop and ask a
human.** Do not invent a workaround and do not copy the shape of the
handler next to it. Every workaround in this area so far has turned out to
be unnecessary, and each one was copied from a neighbour rather than
reasoned about.

## Rationales that are false — check before repeating them

These are quoted from real doc comments in this repo. Every one is wrong,
and each spread by citation ("like `InviteHandler`", "for the same reason
as `CreateSpaceHandler`") long after the original claim stopped being true.

| Claim | Why it is false |
|---|---|
| "needs durable branch state the decoded command doesn't carry" | `Provider::execute(&self, …)` — `&self` **is** the env. `env.state()` reaches operator, reactor, storage. |
| "targets the repo from the origin rather than a command field" | `env.origin()`. |
| "needs the profile handle, the reactor cache, and storage" | All on `env.state()`. |
| "a URL round-trips as `Value::Entity`, so a `String` field never decodes it" | False for the claim path. A field declared `as: text` arrives as `Value::String`. Verified. |
| "native has no clock dependency, so 0" | True only while native meant tests. The CLI dispatches the same handlers; use `crate::clock`, not `js_sys::Date::now()`. |

The one real constraint: **a provider cannot query the transient's own
facts.** The induce sweep retracts the transient at commit and dispatch
runs afterwards (pinned by `transient ping should have been swept`), and on
one path the transient never reaches a branch at all. That is an argument
for **declaring the field**, never for reading raw facts.

## Declaring a field instead of reading facts

An undeclared parameter is not silently coerced — the claim conversion
**fails outright**, so anything riding a transient is declared anyway. Give
it a name and a type:

```yaml
command!: &space/create
  with:
    name:   { the: xyz.tonk.command.create-space/name,   as: text }
    remote: { the: xyz.tonk.command.create-space/remote, as: text }   # arrives as Value::String
```

Then the provider receives it in `C::Input` and no one touches `EntityFacts`.

## Verify a claim before you encode it

Reading type definitions gives a coherent story that can still be wrong.
The `Value::Entity` claim above survived for months because it is true of a
bare `serde_json::from_str::<Value>` — which the claim path never does.
A ten-line probe settled it. When a rationale would justify a workaround,
prove it first, and prefer a contradiction you already have: if the
rationale were true, `Join`'s URL field could not work, and it plainly does.

## See also

- `commands-not-routes` — why an operation is a command rather than an HTTP route.
- `schema-first` — the YAML declaration is the schema; the Rust struct follows it.
