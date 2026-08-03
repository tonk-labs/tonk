---
name: testing
description: How tests work in this repo — Rust crates use #[dialog_common::test] (NOT #[test] or #[tokio::test]) so the same source runs unchanged on native (tokio) and wasm32 (wasm-bindgen-test in a real browser). Use when writing or modifying any Rust test, before adding test deps, or whenever about to type #[test] / #[tokio::test] in a tonk-* crate.
allowed-tools: Read, Bash, Glob, Grep
---

# Testing in this repo

Three layers of tests, each with its own harness. **Default to `#[dialog_common::test]`** — the macro picks the right runtime per target so the same test source runs everywhere.

## Always use `#[dialog_common::test]`, not `#[test]` or `#[tokio::test]`

The macro expands to:

- **Native targets**: `#[tokio::test]` for async functions, `#[test]` for sync.
- **`wasm32-unknown-unknown`**: `#[wasm_bindgen_test]`, runs in headless Chrome via chromedriver.

Same source, both targets. Browser tests need it because plain `#[test]` doesn't run in a browser; native tests benefit because future runtime changes happen in one place.

```rust
// Right
#[dialog_common::test]
async fn it_does_a_thing() {
    let result = some_async_call().await.unwrap();
    assert_eq!(result, 42);
}

// Wrong — won't run on wasm32, won't be picked up by `test:web:debug`
#[tokio::test]
async fn it_does_a_thing() { /* ... */ }
```

### Required dev-deps for any crate using `#[dialog_common::test]`

```toml
[dev-dependencies]
dialog-common = { workspace = true, features = ["helpers"] }
tokio = { workspace = true, features = ["macros", "rt"] }
wasm-bindgen-test = { workspace = true }

[lints.rust]
unexpected_cfgs = { level = "warn", check-cfg = [
    'cfg(dialog_test_native_integration)',
    'cfg(dialog_test_wasm_integration)',
    'cfg(feature, values("web-integration-tests"))',
] }
```

Skip `tokio` if there are zero async tests, skip `wasm-bindgen-test` if the crate doesn't compile to wasm — but the typical crate needs both. The `[lints.rust]` block silences spurious "unexpected cfg" warnings the macro emits per its check-cfg expectations.

### **Always** add `wasm_bindgen_test_configure!` to wasm-runnable test mods

`wasm-bindgen-test` defaults to running tests **in Node.js**, which CI's `test:web:*` environment doesn't have. The test fails with `Error: failed to find or execute Node.js` and 33 other tests get cancelled because of fail-fast.

Every test mod whose tests run on wasm32 must explicitly select a browser-side runner:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn my_test() { /* ... */ }
}
```

**Always include the configure line, even for sync unit tests that don't *look* like they need a runtime.** `dialog_common::test` expands to `#[wasm_bindgen_test]` on wasm regardless, and the runtime selection still applies.

Pick the right host:

- **`run_in_browser`** — when the tests need `window` / `document` / DOM APIs, or when the test body does no I/O at all and you just want the tests to run somewhere. Default for unit tests in `tonk-notation`, `tonk-language-server`, `tonk-invite`, `tonk-code`, `tonk-schema`.
- **`run_in_service_worker`** — when the tests need IndexedDB / Cache API / Storage / `caches` etc. Default for `tonk-worker` router tests.

The `#[cfg(target_arch = "wasm32")]` guards keep the configure line out of the way on native (where it doesn't apply); the test attribute itself dispatches per target so the *tests* don't need any cfg.

**Exception — Cloudflare Worker crates (`tonk-account-service`, `tonk-access-service`): gate the whole mod off wasm instead.** No configure line can save them. Their `#[event(fetch)]` exports a JS function named `fetch`, and the wasm-bindgen harness loads a module by calling `fetch` from inside that same module — where the crate's export shadows the global. The module never initializes: the page hangs on `Loading Wasm module...` and the runner reports `Failed to detect test as having been run`. Write `#[cfg(all(test, not(target_arch = "wasm32")))] mod tests`, which is what every other test mod in those two crates already does.

## Three test layers

### 1. Unit tests (zero arguments)

Live next to the code in `#[cfg(test)] mod tests` blocks. Run with the macro's basic mode — `#[dialog_common::test] fn` (sync) or `async fn` (async).

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[dialog_common::test]
    async fn parses_yaml() {
        let parsed = parse("a: 1");
        assert!(parsed.diagnostics.is_empty());
    }
}
```

### 2. Wasm-only DOM/SW tests

Live in the same `#[cfg(test)] mod tests` blocks but gated to wasm32. Use `wasm_bindgen_test_configure!` to choose the JS execution context:

- `run_in_browser` — needs `window` / `document` / DOM APIs (e.g. tonk-invite, tonk-code).
- `run_in_service_worker` — needs IndexedDB / Cache API / Storage etc. (e.g. tonk-worker).

```rust
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    wasm_bindgen_test_configure!(run_in_service_worker);

    use crate::worker::TonkState;

    #[dialog_common::test]
    async fn it_does_browser_things() {
        // ...
    }
}
```

These run via `nix develop -c test:web:debug` (or `:release`), which builds a wasm test archive and drives Chrome through chromedriver.

### 3. Real-browser integration tests (chromedriver-driven)

For testing the **real built app** — the actual `tonk-ui` SPA, the actual `<tonk-code>` editor bundle, real LSP wiring through Caddy + the access service. These take a `TestEnvironment` parameter from `tonk_ui::helpers`:

```rust
#[dialog_common::test]
async fn it_does_a_real_thing(env: TestEnvironment) -> Result<()> {
    let driver = env.driver().await?;

    // Wait for something via CSS selector
    driver.query(By::Css(".space-banner-title")).first().await?;

    // Or eval JS in the page
    let result = driver.execute(
        r#"const el = document.querySelector("tonk-code");
           el.value = "hello";
           return el.value;"#,
        vec![],
    ).await?;
    assert_eq!(result.json().as_str(), Some("hello"));

    driver.quit().await?;
    Ok(())
}
```

The macro detects "function takes a parameter" and generates an *integration* test instead of a unit test. The test:

- Spins up Caddy serving the prebuilt `tonk-ui` (with all assets including `tonk-code/*`).
- Spins up the access service for `/ucan/*` proxying.
- Spins up chromedriver.
- Hands you a `TestEnvironment` with the URLs.

Examples in `rust/tonk-ui/src/identity.rs` and `rust/tonk-ui/src/account_flow.rs`.

This is the right harness whenever the test needs to verify *real bundle behavior* — CodeMirror diagnostics rendering, LSP round-tripping, service-worker registration, full SPA navigation. Don't try to fake the bundle with stubs; use the real one.

## How to run

In the dev shell (`nix develop`):

| Command | What it runs |
|---|---|
| `test:native:debug` | Workspace minus `tonk-ui` and `tonk-core`, native target, `integration-tests` feature. |
| `test:native:release` | Same, release profile. |
| `test:web:debug` | Workspace, `wasm32-unknown-unknown`, headless Chrome via chromedriver. |
| `test:web:release` | Same, release profile. |
| `test:all` | All four native/wasm configurations. |
| `test:e2e` | Serialized real-browser `tonk-ui` tests, including account-service and CLI roundtrips. |

`tonk-ui`'s real-browser integration tests are excluded from the four default workspace test runs (the flake explicitly `--exclude tonk-ui`). Run them with `nix develop -c test:e2e`; CI runs the same command in its dedicated e2e job.

## Common patterns

### Setting up a router test (worker)

```rust
#[cfg(all(test, target_arch = "wasm32", target_os = "unknown"))]
mod tests {
    wasm_bindgen_test_configure!(run_in_service_worker);

    #[dialog_common::test]
    async fn it_returns_something() {
        let state = test_state().await;
        let (app, _lsp) = api_router(state);
        let response = app
            .oneshot(Request::builder()
                .uri("/api/...")
                .body(Body::empty()).unwrap())
            .await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }
}
```

`test_state()` and `put_repo()` are convenience helpers in `tonk-worker/src/router.rs::tests` — copy the pattern when adding to that suite.

### Writing a real-browser test

```rust
#[dialog_common::test]
async fn it_drives_the_editor(env: TestEnvironment) -> Result<()> {
    let driver = env.driver().await?;

    // Wait for the page to be ready
    driver.query(By::Css(".space-banner-title")).first().await?;

    // Find the editor element
    let editor = driver.query(By::Css("tonk-code")).first().await?;

    // Drive it
    let value = driver.execute(
        r#"
        const el = arguments[0];
        el.value = "person-name:\n  attribute:\n    the: x/y\n";
        return el.value;
        "#,
        vec![editor.to_json()?],
    ).await?;

    assert!(value.json().as_str().unwrap_or("").contains("person-name"));

    driver.quit().await?;
    Ok(())
}
```

## Anti-patterns

- **`#[tokio::test]` directly** — won't run on wasm. Use `#[dialog_common::test]`.
- **`#[wasm_bindgen_test]` directly** — won't run on native. Same.
- **`futures::executor::block_on(async_call())` inside `#[test]`** — the test isn't async. Make it `#[dialog_common::test] async fn` and `.await` instead.
- **Stub bundles or mocks for testing the real `<tonk-code>` editor** — use a real-browser integration test in `tonk-ui`. Stubs verify your own contract; they don't catch CodeMirror/LSP bugs.
- **JS-side tests for things that have a Rust integration-test path** — prefer the Rust path so coverage shows up in the same `nix develop -c test:*` runs.

## Where to look for examples

| Pattern | Look at |
|---|---|
| Sync unit test | `tonk-notation/src/parse.rs::tests` |
| Async unit test | `tonk-schema/src/interpret.rs::tests` |
| Wasm-in-service-worker test | `tonk-worker/src/router.rs::tests` (the entire mod) |
| Wasm-in-browser test | `tonk-invite/src/lib.rs::tests` |
| Real-browser integration test | `tonk-ui/src/identity.rs`, `tonk-ui/src/account_flow.rs` |
| Provider machinery for the integration tests | `tonk-ui/src/helpers.rs` (`TestEnvironment`, `TestServers`) |
| Flake test commands | `flake.nix` `commands.test:*` |
