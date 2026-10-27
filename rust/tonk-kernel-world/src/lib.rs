//! Dialog DB over the world `sapling:kernel`, for a Tonk kernel on WASI.
//!
//! A kernel holds no storage and no key of its own: its host gives it the
//! layers of its stack and a session, and holds every key. These are the
//! providers and signers that let Dialog DB, and Tonk's worker over it, run on
//! them:
//!
//! - [`layer`]: a layer, as the world serves it, and [`guest::HandleLayer`]
//!   over a handle of the world.
//! - [`store`]: the archive, the memory and the blobs of a space, over a
//!   layer. [`mount`] opens one from a location.
//! - [`session`]: the session the boot hands over.
//! - [`credentials::signer`]: a credential, as a signer whose signatures come
//!   from the host.
//! - [`space`]: a space's credential store, as custodies and secrets in its
//!   cells.
//! - [`stack`]: the layer of the worker's own, which every location it
//!   opens is a part of.
//!
//! On every other target the crate holds only [`boot`] and [`naming`], what
//! the host and the person's profile read too.

pub mod boot;
pub mod naming;

#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod world {
    wit_bindgen::generate!({
        path: "wit",
        world: "sapling:kernel/imports",
        generate_all,
    });
}

#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod cache;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod credentials;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod guest;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod layer;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod mount;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod session;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod signal;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod space;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod stack;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod store;
