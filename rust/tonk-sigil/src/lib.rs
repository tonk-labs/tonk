mod sigil;

pub use sigil::Sigil;

pub mod did;
pub use did::{did_key_prefix, did_sigil_value};

#[cfg(feature = "web")]
mod web;
