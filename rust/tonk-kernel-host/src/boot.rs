//! What a space holds to boot: its kernel, as a blob of the space, and the
//! boot cell that pins it.
//!
//! The boot cell is `boot/loader`. Its content is a DAG-CBOR map,
//! `{ "v": 2, "kernel": <id> }`, where the id is the content identifier of
//! the kernel's wasm (CIDv1, raw codec, BLAKE3, base32). The wasm is the
//! space's blob under that BLAKE3 hash. A space that holds both runs that
//! kernel, and needs nothing from anywhere else to start it.
//!
//! Which space an origin's host serves is a cell of the origin's own:
//! `boot/subject`, in the layer [`ORIGIN`] names.

pub use tonk_kernel_world::boot::{CELL, NAMESPACE, kernel_id};
use tonk_kernel_world::boot::{cell_content, digest, read_cell};
use tonk_kernel_world::naming::layer_name;

use dialog_effects::storage::{Directory, Location};

use crate::layers::{self, LayerSpace, Layers};

/// Where an origin's host keeps which space it serves.
pub const ORIGIN: &str = "tonk.origin";

/// The cell of [`ORIGIN`] that holds the subject of the space it serves.
const SUBJECT: &str = "subject";

fn failed(error: impl core::fmt::Debug) -> String {
    format!("{error:?}")
}

async fn origin(layers: &Layers) -> Result<LayerSpace, String> {
    layers
        .open(&layer_name(&Location::new(Directory::Profile, ORIGIN)))
        .await
        .map_err(failed)
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

/// Say that this origin's host serves the space `subject`.
pub async fn hold(layers: &Layers, subject: &str) -> Result<(), String> {
    let origin = origin(layers).await?;
    let current = layers::resolve(&origin, NAMESPACE, SUBJECT)
        .await
        .map_err(failed)?;
    layers::publish(
        &origin,
        NAMESPACE,
        SUBJECT,
        subject.as_bytes().to_vec(),
        current.map(|edition| edition.version),
    )
    .await
    .map_err(failed)?;
    Ok(())
}

/// The space this origin's host serves, if it serves one yet.
pub async fn held(layers: &Layers) -> Result<Option<String>, String> {
    let origin = origin(layers).await?;
    let edition = layers::resolve(&origin, NAMESPACE, SUBJECT)
        .await
        .map_err(failed)?;
    edition
        .map(|edition| String::from_utf8(edition.content).map_err(failed))
        .transpose()
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

    #[tokio::test]
    async fn it_remembers_the_space_its_origin_serves() {
        let layers = scratch("held");
        assert_eq!(held(&layers).await.unwrap(), None);
        hold(&layers, SPACE).await.unwrap();
        assert_eq!(held(&layers).await.unwrap(), Some(SPACE.to_owned()));
    }
}
