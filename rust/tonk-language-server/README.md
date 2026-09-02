# tonk-language-server

A transport-agnostic Language Server Protocol (LSP) implementation for `tonk-notation` documents.

This crate sits between [`tonk-notation`](../tonk-notation) (the pure language model), [`tonk-analyzer`](../tonk-analyzer) (resolution and diagnostics), and whatever runtime hosts the server (the in-browser service worker driving CodeMirror today, a stdio CLI tomorrow). It owns the JSON-RPC envelope, per-document text state, capability negotiation, and a server-push channel, and it resolves diagnostics, completion, and hover against the live branch the document belongs to. The crate is synchronous and runtime-agnostic: no `tokio`, no I/O. Hosts pump bytes through [`Server::handle_message`] and drain server-initiated notifications with [`Server::take_outbound`].

## LSP features

Capabilities advertised from `initialize` (see `server_capabilities`):

- **Text document sync**: `FULL`. Every `didChange` carries the whole buffer; the server replaces the stored text.
- **Push diagnostics**: the server emits `textDocument/publishDiagnostics` on every open/change. The pull-diagnostic capability is intentionally not advertised.
- **Completion**: triggered on newline (head vs field surface, decided by cursor indent) and `?` (logic variables / anchors). `:` is deliberately not a trigger.
- **Hover**: `Simple(true)`. Concept descriptions in head position; backing-attribute description, type, and cardinality for body fields.
- **Position encoding**: UTF-16, the LSP default and what `@codemirror/lsp-client` sends.

Methods handled (see `Server::handle_request` / `handle_notification`):

- Lifecycle: `initialize`, `initialized`, `shutdown`, `exit`.
- Documents: `textDocument/didOpen`, `textDocument/didChange`, `textDocument/didClose`.
- Requests: `textDocument/completion`, `textDocument/hover`.
- Server-pushed: `textDocument/publishDiagnostics`.

The JSON-RPC 2.0 envelope (`Request` / `Notification` / `Response`, reserved error codes) is modeled in `jsonrpc.rs`, since `lsp-types` covers each method's `params` and `result` but not the framing.

## Diagnostics

`publish` runs two passes and always emits a `publishDiagnostics` frame (empty list included, so clients learn an error was resolved). The frame echoes the document version so `@codemirror/lsp-client` can discard stale frames whose ranges no longer match its buffer.

1. **Parser**: `tonk_notation::parse` returns structural diagnostics (unbalanced indentation, missing colon, etc.). The parser is permissive and still yields a `Syntax` tree on recoverable errors.
2. **Analyzer**: only when the parser is clean. `tonk_analyzer::analyzer::scan_variables` always runs (the structural variable-occurrence scan, e.g. lone `?` warnings). When the host opened a live environment, `tonk_analyzer::analyzer::analyze` additionally runs against the branch to surface branch-dependent verdicts (`UnknownConcept`, etc.). Without an environment the analyzer pass is document-only and branch-dependent errors are filtered out.

Analyzer diagnostics carry a stable `E_…` / `W_…` code and a source range (clamped against the live text), tagged with source `tonk-schema`.

## Completion and hover

Both are position-driven: the cursor's place in the line (column-zero head vs indented body, presence of `?`, a premise's `assert:` value) decides what is offered, not just the trigger character. Built-ins always resolve from `tonk_schema::builtin` (the concept registry) and `tonk_analyzer` (formula and constraint registries). When the host opens a live `(source, env)` pair, branch-published concepts and named references are folded in via `tonk_analyzer` / `tonk_schema` resolution chains (`NamedReference::list`, `ConceptDefinition::list`, `ConceptReference::resolve`).

## The host seam

The language server never knows how an environment is opened. It defines the port; the host implements it. Per request the server parses the document URI (`tonk-buffer:///<encoded-repo>/<encoded-branch>/…`) to `(repo, branch)` and calls [`EnvProvider::open`], which returns an [`Opened`] holding a [`Source`] (the branch or transaction overlay) and a `QueryEnv` for the request's lifetime. Repository/profile and branch identities use the shared canonical segment codec, so a legal branch such as `feat/artifact` is carried as `feat%2Fartifact` without changing URI or HTTP route structure; non-canonical aliases are rejected. The LSP threads `opened.source()` into each resolution chain and `opened.env()` into its `.perform(env)`, matching the dialog command/perform idiom.

[`NoEnv`] is the no-host provider (`open` always returns `None`): tests and a standalone editor pass it, and the server then resolves only the document's own declarations.

## Hosting

The server is `!Sync` and meant to live behind whatever lock the host provides (`Rc<RefCell<…>>` in single-threaded wasm, `Arc<Mutex<…>>` in a tokio host). The browser service worker dispatches a single `axum` handler that calls [`Server::handle_message`], serializes the reply, and forwards drained outbound notifications over its push channel.

## Dependencies

`lsp-types` for protocol types, `serde` / `serde_json` for the wire format, `tonk-notation` for parsing, `tonk-analyzer` for analysis, and `tonk-schema` for the built-in registries and resolution chains. `dialog-capability` / `dialog-common` supply the `Command`/`Provider` and `ConditionalSync` plumbing the env seam builds on. No async runtime is a dependency.
