//! Deterministic friendly names for members.
//!
//! `petname` maps a profile DID to a stable `adjective-animal` pair. It is
//! the default display name a member gets before they pick one. Pure and
//! deterministic: the same DID always yields the same name (no RNG), so a
//! member reads the same on every device without storing the default.

use dialog_varsig::Did;

const ADJECTIVES: &[&str] = &[
    "fancy", "brave", "calm", "clever", "eager", "gentle", "jolly", "keen",
    "lively", "mellow", "nimble", "plucky", "quiet", "rapid", "sunny", "witty",
    "bold", "bright", "cosy", "deft", "fond", "glad", "honest", "lucky",
    "merry", "noble", "proud", "quirky", "spry", "tidy", "vivid", "warm",
];

const ANIMALS: &[&str] = &[
    "otter", "lynx", "heron", "marten", "badger", "vole", "finch", "newt",
    "stoat", "ibis", "tapir", "gecko", "raven", "quokka", "dingo", "okapi",
    "panda", "robin", "shrew", "toad", "wren", "yak", "zebra", "puffin",
    "mole", "hare", "swift", "crane", "loris", "civet", "fossa", "genet",
];

/// A stable `adjective-animal` name derived from a profile DID.
pub fn petname(did: &Did) -> String {
    // Two independent FNV-1a folds over the DID string give two indices.
    // FNV is stable across platforms and runs (unlike `DefaultHasher`,
    // which is randomized), preserving the determinism contract.
    let bytes = did.as_str().as_bytes();

    let adj = fnv1a(bytes, 0xcbf29ce484222325) as usize % ADJECTIVES.len();
    let animal = fnv1a(bytes, 0x100000001b3) as usize % ANIMALS.len();
    format!("{}-{}", ADJECTIVES[adj], ANIMALS[animal])
}

fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
    let mut hash = seed;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::petname;
    use dialog_varsig::Did;

    fn did(seed: &str) -> Did {
        format!("did:key:{seed}").parse().unwrap()
    }

    #[test]
    fn it_is_deterministic_for_the_same_did() {
        let d = did("z6MkProfileA");
        assert_eq!(petname(&d), petname(&d));
    }

    #[test]
    fn it_produces_an_adjective_animal_pair() {
        let name = petname(&did("z6MkProfileA"));
        let parts: Vec<&str> = name.split('-').collect();
        assert_eq!(parts.len(), 2, "expected adjective-animal, got {name}");
        assert!(parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase())));
    }

    #[test]
    fn it_differs_for_different_dids() {
        // Not a hard guarantee, but the two chosen seeds must not collide.
        assert_ne!(petname(&did("z6MkProfileA")), petname(&did("z6MkProfileB")));
    }
}
