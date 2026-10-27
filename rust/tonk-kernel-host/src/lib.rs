//! A spike: run a Sapling guest kernel on the Wasm Component Model Polyfill,
//! natively on Wasmtime's core engine and in a browser on its own.

pub mod boot;
pub mod credentials;
#[cfg(target_arch = "wasm32")]
pub mod embedder;
pub mod http_types;
pub mod kernel;
pub mod layers;
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

/// The delegations a guest kernel built for `persona` grants: an export a
/// host acquires only from a kernel that has it.
pub mod persona_bindings {
    wcmp::bindgen!({
        path: "wit",
        world: "sapling:kernel/persona-grant",
        exports: { default: async | store },
        with: {
            "sapling:kernel/types@0.1.0": crate::bindings::sapling::kernel::types,
            "sapling:kernel/credential@0.1.0": crate::bindings::sapling::kernel::credential,
            "sapling:kernel/session@0.1.0": crate::bindings::sapling::kernel::session,
        },
    });
}

pub mod wasi_bindings {
    wcmp::bindgen!({
        path: "wit-wasi",
        world: "spike:wasi/rest",
        imports: { "wasi:cli/stdin.read-via-stream": store },
    });
}
