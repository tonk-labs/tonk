//! A spike: run a Sapling guest kernel on the Wasm Component Model Polyfill,
//! natively on Wasmtime's core engine and in a browser on its own.

#[cfg(target_arch = "wasm32")]
pub mod embedder;
pub mod http_types;
pub mod kernel;
pub mod platform;
pub mod sapling;
pub mod wasi;
pub mod wasi_rest;

pub mod bindings {
    wcmp::bindgen!({
        path: "wit",
        world: "sapling:kernel/host",
        exports: { default: async | store },
    });
}

pub mod wasi_bindings {
    wcmp::bindgen!({
        path: "wit-wasi",
        world: "spike:wasi/rest",
        imports: { "wasi:cli/stdin.read-via-stream": store },
    });
}
