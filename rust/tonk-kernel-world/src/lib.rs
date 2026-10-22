//! Dialog DB over the world `sapling:kernel`, for a Tonk kernel on WASI.
//!
//! A kernel holds no storage and no key of its own: its host gives it the
//! layers of its stack, signs heads and revision records with its session,
//! and keeps the credentials of each layer (PDD015). These are the providers
//! and signers that let Dialog DB, and Tonk's worker over it, run on them:
//!
//! - [`layer`]: a layer, as the world serves it, and [`guest::HandleLayer`]
//!   over a handle of the world.
//! - [`store`]: the archive, the memory and the blobs of a space, over a
//!   layer. [`mount`] opens one from a location.
//! - [`guest::session_signer`] and [`credentials::signer`]: the session key and
//!   a credential, as signers whose signatures come from the host.
//!
//! On every other target the crate holds only [`boot`], the boot cell's
//! codec, which the host reads too.

pub mod boot;

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
pub mod signal;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod space;
#[cfg(all(target_arch = "wasm32", not(target_os = "unknown")))]
pub mod store;
