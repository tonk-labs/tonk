//! What a space holds to boot: its kernel, as a blob of the space, and the
//! boot cell that pins it.
//!
//! The boot cell is `boot/loader`. Its content is a DAG-CBOR map,
//! `{ "v": 2, "kernel": <id> }`, where the id is the content identifier of
//! the kernel's wasm (CIDv1, raw codec, BLAKE3, base32). The wasm is the
//! space's blob under that BLAKE3 hash. A space that holds both runs that
//! kernel, and needs nothing from anywhere else to start it.

pub use tonk_kernel_world::boot::{CELL, NAMESPACE, kernel_id};
use tonk_kernel_world::boot::{cell_content, digest, read_cell};

use crate::bindings::sapling::kernel::storage::Edition;
use crate::sapling::Change;

/// The writes that install `kernel` into the space `subject`: the kernel as
/// a blob, and the boot cell pinning it. Answers the kernel's id with them.
pub fn install(subject: &str, kernel: &[u8]) -> (String, Vec<Change>) {
    let id = kernel_id(kernel);
    let content = cell_content(&id);
    let version = blake3::hash(&content).as_bytes().to_vec();
    let changes = vec![
        Change::Blob {
            subject: subject.to_owned(),
            key: blake3::hash(kernel).as_bytes().to_vec(),
            bytes: kernel.to_vec(),
        },
        Change::Cell {
            subject: subject.to_owned(),
            namespace: NAMESPACE.to_owned(),
            cell: CELL.to_owned(),
            edition: Some(Edition { content, version }),
        },
    ];
    (id, changes)
}

/// The space whose boot cell the store holds, among what it holds: the space
/// a host that kept them serves.
pub fn held(held: &[Change]) -> Option<String> {
    held.iter().find_map(|change| match change {
        Change::Cell {
            subject,
            namespace,
            cell,
            edition: Some(_),
        } if namespace == NAMESPACE && cell == CELL => Some(subject.clone()),
        _ => None,
    })
}

/// The kernel the space `subject` pins, as its id and its bytes, from what
/// the space's store holds. `None` when the space pins none. An error when it
/// pins one it does not hold, or holds bytes that are not the ones pinned.
pub fn pinned(subject: &str, held: &[Change]) -> Result<Option<(String, Vec<u8>)>, String> {
    let mut boot = None;
    for change in held {
        if let Change::Cell {
            subject: of,
            namespace,
            cell,
            edition,
        } = change
            && of == subject
            && namespace == NAMESPACE
            && cell == CELL
        {
            boot = edition.as_ref();
        }
    }
    let Some(boot) = boot else {
        return Ok(None);
    };
    let id = read_cell(&boot.content)?;
    let digest = digest(&id)?;
    let kernel = held
        .iter()
        .rev()
        .find_map(|change| match change {
            Change::Blob {
                subject: of,
                key,
                bytes,
            } if of == subject && key.as_slice() == digest => Some(bytes),
            _ => None,
        })
        .ok_or_else(|| format!("the space pins the kernel {id} and does not hold it"))?;
    if blake3::hash(kernel).as_bytes() != &digest {
        return Err(format!("the space holds other bytes than the kernel {id}"));
    }
    Ok(Some((id, kernel.clone())))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: &str = "did:key:z6MkSpace";

    #[test]
    fn it_boots_the_kernel_it_installed() {
        let (id, changes) = install(SPACE, b"kernel");
        assert_eq!(
            pinned(SPACE, &changes).unwrap(),
            Some((id, b"kernel".to_vec()))
        );
    }

    #[test]
    fn it_pins_nothing_for_a_space_with_no_boot_cell() {
        let (_, changes) = install(SPACE, b"kernel");
        assert_eq!(pinned("did:key:z6MkOther", &changes).unwrap(), None);
    }

    #[test]
    fn it_refuses_a_pin_whose_kernel_the_space_does_not_hold() {
        let (_, changes) = install(SPACE, b"kernel");
        let error = pinned(SPACE, &changes[1..]).unwrap_err();
        assert!(error.contains("does not hold it"), "{error}");
    }

    #[test]
    fn it_finds_the_space_whose_kernel_it_holds() {
        let (_, changes) = install(SPACE, b"kernel");
        assert_eq!(held(&changes), Some(SPACE.to_owned()));
        assert_eq!(held(&changes[..1]), None, "a blob alone pins nothing");
    }

    #[test]
    fn it_writes_the_same_boot_cell_for_the_same_kernel() {
        let (_, first) = install(SPACE, b"kernel");
        let (_, second) = install(SPACE, b"kernel");
        let content = |changes: &[Change]| match &changes[1] {
            Change::Cell {
                edition: Some(edition),
                ..
            } => edition.content.clone(),
            other => panic!("not the boot cell: {other:?}"),
        };
        assert_eq!(content(&first), content(&second));
        assert_eq!(content(&first)[..4], [0xa2, 0x61, b'v', 0x02]);
    }
}
