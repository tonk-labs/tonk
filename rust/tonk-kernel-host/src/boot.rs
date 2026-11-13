//! What a space holds to boot: its kernel, as a blob of the space, and the
//! boot cell that pins it.
//!
//! The boot cell is `boot/loader`. Its content is a DAG-CBOR map,
//! `{ "v": 2, "kernel": <id> }`, where the id is the content identifier of
//! the kernel's wasm (CIDv1, raw codec, BLAKE3, base32). The wasm is the
//! space's blob under that BLAKE3 hash. A space that holds both runs that
//! kernel, and needs nothing from anywhere else to start it.
//!
//! The space an origin's host serves is the one whose layer holds a boot
//! cell: the host keeps no record of its own of which it is.

pub use tonk_kernel_world::boot::{CELL, NAMESPACE, kernel_id};
use tonk_kernel_world::boot::{cell_content, digest, read_cell};
use wasmparser::{Parser, Payload};

use crate::layers::{self, Layers};

fn failed(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}

/// Install `kernel` into the space `subject`: the kernel as a blob of the
/// space, and the boot cell pinning it. Answers the kernel's id.
pub async fn install(layers: &Layers, subject: &str, kernel: &[u8]) -> Result<String, String> {
    let space = layers.open(subject).await.map_err(failed)?;
    let id = kernel_id(kernel);
    layers::write(&space, kernel).await.map_err(failed)?;
    let current = layers::resolve(&space, NAMESPACE, CELL)
        .await
        .map_err(failed)?;
    layers::publish(
        &space,
        NAMESPACE,
        CELL,
        cell_content(&id),
        current.map(|edition| edition.version),
    )
    .await
    .map_err(failed)?;
    Ok(id)
}

/// The kernel the space `subject` pins, as its id and its bytes, from what
/// the space holds. `None` when the space pins none. An error when it pins
/// one it does not hold, or holds bytes that are not the ones pinned.
pub async fn pinned(layers: &Layers, subject: &str) -> Result<Option<(String, Vec<u8>)>, String> {
    let space = layers.open(subject).await.map_err(failed)?;
    let Some(boot) = layers::resolve(&space, NAMESPACE, CELL)
        .await
        .map_err(failed)?
    else {
        return Ok(None);
    };
    let id = read_cell(&boot.content)?;
    let digest = digest(&id)?;
    let kernel = match layers::read(&space, &digest, 0, None).await {
        Ok(kernel) => kernel,
        Err(_) => {
            return Err(format!(
                "the space pins the kernel {id} and does not hold it"
            ));
        }
    };
    if blake3::hash(&kernel).as_bytes() != &digest {
        return Err(format!("the space holds other bytes than the kernel {id}"));
    }
    Ok(Some((id, kernel)))
}

/// The interface only a persona exports: the world `sapling:kernel/persona`
/// is `kernel` with it. What a persona answers through it, delegating to the
/// person's other workers and acting for the person, is what a site whose
/// kernel is one can do and another cannot.
const GUEST: &str = "sapling:kernel/guest@";

/// Whether the kernel in `kernel` is a persona: it exports `guest`, at the
/// top of the component. Bytes that do not parse are no persona.
pub fn is_persona(kernel: &[u8]) -> bool {
    let mut depth = 0usize;
    for payload in Parser::new(0).parse_all(kernel) {
        match payload {
            Err(_) => return false,
            Ok(Payload::ModuleSection { .. } | Payload::ComponentSection { .. }) => depth += 1,
            Ok(Payload::End(_)) => depth = depth.saturating_sub(1),
            Ok(Payload::ComponentExportSection(exports)) if depth == 0 => {
                for export in exports {
                    match export {
                        Ok(export) if export.name.name.starts_with(GUEST) => return true,
                        Ok(_) => {}
                        Err(_) => return false,
                    }
                }
            }
            Ok(_) => {}
        }
    }
    false
}

/// The space this origin's host serves: the one whose layer holds a boot
/// cell, if any does yet.
pub async fn held(layers: &Layers) -> Result<Option<String>, String> {
    for subject in layers.subjects().await.map_err(failed)? {
        let space = layers.open(&subject).await.map_err(failed)?;
        if layers::resolve(&space, NAMESPACE, CELL)
            .await
            .map_err(failed)?
            .is_some()
        {
            return Ok(Some(subject));
        }
    }
    Ok(None)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    const SPACE: &str = "did:key:z6MkSpace";

    fn scratch(name: &str) -> Layers {
        let path = std::env::temp_dir().join(format!(
            "tonk-kernel-host-boot-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Layers::under(path.to_string_lossy().into_owned())
    }

    #[tokio::test]
    async fn it_boots_the_kernel_it_installed() {
        let layers = scratch("installed");
        let id = install(&layers, SPACE, b"kernel").await.unwrap();
        assert_eq!(
            pinned(&layers, SPACE).await.unwrap(),
            Some((id, b"kernel".to_vec()))
        );
    }

    #[tokio::test]
    async fn it_pins_nothing_for_a_space_with_no_boot_cell() {
        let layers = scratch("none");
        install(&layers, SPACE, b"kernel").await.unwrap();
        assert_eq!(pinned(&layers, "did:key:z6MkOther").await.unwrap(), None);
    }

    #[tokio::test]
    async fn it_takes_up_another_kernel_in_place_of_the_one_it_pinned() {
        let layers = scratch("replaced");
        install(&layers, SPACE, b"kernel").await.unwrap();
        let id = install(&layers, SPACE, b"another").await.unwrap();
        assert_eq!(
            pinned(&layers, SPACE).await.unwrap(),
            Some((id, b"another".to_vec()))
        );
    }

    #[test]
    fn it_tells_a_persona_by_what_it_exports() {
        assert!(is_persona(tonk_components::TONK_PERSONA_BYTES));
        assert!(!is_persona(tonk_components::TONK_KERNEL_BYTES));
        assert!(!is_persona(b"kernel"));
    }

    #[tokio::test]
    async fn it_serves_the_space_whose_kernel_it_holds() {
        let layers = scratch("held");
        assert_eq!(held(&layers).await.unwrap(), None);
        install(&layers, SPACE, b"kernel").await.unwrap();
        assert_eq!(held(&layers).await.unwrap(), Some(SPACE.to_owned()));
    }
}
