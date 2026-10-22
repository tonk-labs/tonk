//! The components that the Tonk build makes for the world `sapling:kernel`,
//! each built for `wasm32-wasip3` by the build script of this crate, as
//! Sapling builds its guest kernels.
//!
//! [`TONK_KERNEL_BYTES`] is the Tonk kernel: Tonk's worker as a component (see
//! `rust/tonk-kernel`). It is bytes in the binary, not a path of the build
//! directory, so a test runs from an archive of the build.

include!(concat!(env!("OUT_DIR"), "/components.rs"));
