//! The kernel a space runs, on the space's own `main`.

use dialog_artifacts::Entity;
use dialog_query::Concept;

use crate::domain::space::Kernel;

/// The kernel `this` space runs: the asset of its wasm. Written in the
/// space's first commit, with the asset itself, so the space holds what it
/// needs to start wherever it syncs.
#[derive(Concept, Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SpaceKernel {
    /// The space, by its subject DID.
    pub this: Entity,
    /// The asset of the kernel's wasm, `asset:<hash>`.
    pub kernel: Kernel,
}

impl SpaceKernel {
    /// `space` runs the kernel whose asset is `kernel`.
    pub fn new(space: Entity, kernel: Entity) -> Self {
        Self {
            this: space,
            kernel: Kernel(kernel),
        }
    }
}
