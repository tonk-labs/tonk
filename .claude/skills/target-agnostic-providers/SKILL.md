---
name: target-agnostic-providers
description: A command's behaviour belongs in a `Provider<C>` impl that compiles for every target, not in a hand-written handler behind `cfg(target_arch = "wasm32")`. Use whenever adding or editing a command handler, reaching for a `cfg(target_arch)` / `cfg(target_os)` gate to make something compile, or noticing that a command works in the browser but not the CLI or TUI.
allowed-tools: Read, Bash, Glob, Grep
---

# Providers, target-agnostic, or ask

A command names an intent. **The runtime provides and fulfils it.** That
split is the whole point: the same `space/create` a page asserts is the
one the CLI asserts, and the one a TUI keypress asserts. Only the
*provider* differs — a browser does the passkey dance, a CLI does some
filesystem bookkeeping — and abstracting that difference is what
`Provider<C>` is for.

`CommandEnv`'s own doc already states the design:

> Capability is structural: a command runs iff `CommandEnv: Provider<C>`
> is implemented. Registering a command requires that bound, so an
> unsupported command won't even register.

Write code that makes that true.

## The rule

1. **Behaviour goes in `impl Provider<C> for <Env>`**, registered with
   `registry.command::<C>()` (or `registry.migrated::<Current, Legacy>()`
   when an older shape must keep decoding). A hand-written
   `impl CommandHandler<Env>` is the exception, and an exception needs a
   doc comment saying which capability it needs that the env cannot
   supply.

2. **The command type and its provider compile for every target.** Not
   "compiles for wasm and is absent natively". A command that exists in
   the browser and not on the CLI is a command the CLI cannot run, and
   every non-browser host — `tonk` the binary, a TUI, a test — silently
   does nothing.

3. **`cfg(target_arch)` / `cfg(target_os)` around a handler is a smell,
   not a solution.** It does not make the code portable; it makes the
   *absence* portable. If a provider genuinely needs a browser API, put
   the gate around the smallest possible leaf — the one call — behind a
   trait the other host also implements. Never around the registration,
   the handler, or the module.

4. **When it will not go target-agnostic, stop and ask a human.** Say
   which capability resists abstraction and what the alternatives cost.
   Do not gate it, do not stub it, do not leave the native path empty and
   move on. A workaround here is invisible: nothing fails to compile,
   no test goes red, and the feature is simply missing on every host you
   did not build for.

## How to tell it went wrong

```console
# Every handler the worker registers, and what it is gated on.
rg -n 'registry\.(register|command::<|migrated::<)' rust/tonk-worker/src/router/command.rs
rg -c 'cfg\(all\(target_arch = "wasm32"' rust/tonk-worker/src/router/
```

A `command_registry()` whose non-wasm arm returns `CommandRegistry::new()`
means **no command runs natively at all**. That is the failure this skill
exists to prevent, and it is not hypothetical — it is what the worker did
while `commands-not-routes` claimed the opposite.

## Why "ask" rather than "work around"

The workaround is always cheaper in the moment and always wrong, because
the cost lands on a host nobody is testing yet. Asking costs one message.
