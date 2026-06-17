# tonk-common

Cross-platform utilities shared across the tonk crates.

A small grab-bag of helpers that need to behave the same on native and on
`wasm32-unknown-unknown`. Each crate in the workspace pulls from here rather
than reimplementing the same target-conditional plumbing.

## What it provides

- **`log!` macro.** A cross-platform logging macro that expands to
  `web_sys::console::log_1` on `wasm32-unknown-unknown` and `println!` on
  native. Takes the same format arguments as `println!`.

  ```rust
  use tonk_common::log;

  log!("Hello, world!");
  log!("Value: {}", 42);
  log!("Multiple values: {} and {}", "foo", "bar");
  ```

- **`Exclusive<T>`.** A `#[repr(transparent)]` wrapper that grants only unique
  (mutable) access to its inner value and is therefore unconditionally `Send`
  and `Sync`. Use it to make a `!Sync` value usable across `Sync` bounds in
  cases where shared access is never possible. (Adapted from
  async-compression's `Unshared`.)

- **`ExclusiveStream<T>`.** Wraps an `Exclusive<T>` whose inner value is a
  `Stream + Unpin` and implements `Stream` for it, turning a `!Sync` stream
  into a `Sync` one when it is known not to be shared by concurrent actors.
