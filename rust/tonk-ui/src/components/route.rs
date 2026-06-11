//! Shared parsing for the `{branch}@{label}:{id}` space segment.
//!
//! Every route's space segment is a single `:space` param that encodes a
//! repository's id, an optional human-readable label, and an optional
//! branch:
//!
//! - `z6MkABC`            → key `did:key:z6MkABC`, branch `main`
//! - `home:z6MkABC`       → key `did:key:z6MkABC`, branch `main`
//! - `feat@home:z6MkABC`  → key `did:key:z6MkABC`, branch `feat`
//!
//! A repository's identity is its credential's `did:key`. The URL
//! carries only the trailing id (the multibase blob after `did:key:`)
//! plus an optional cosmetic `{label}:` prefix, to keep URLs short; the
//! routing key reconstructs the full `did:key:` + id, which is what
//! names the database, the reactor cache, and the `<tonk-repository>`
//! routing context. `@` separates an optional leading branch; within the
//! rest, the id is everything after the LAST `:` (a bare segment with no
//! `:` is itself the id). Branch names may contain `:` and `/` but not
//! `@`, so the branch split is unambiguous. Centralizing the parse keeps
//! the convention identical across the display, concept, board, and
//! space-viewer routes.

/// The default branch when the space segment names none.
pub const DEFAULT_BRANCH: &str = "main";

/// The `did:key:` prefix the URL id reconstructs into the full DID.
const DID_KEY_PREFIX: &str = "did:key:";

/// A parsed space segment: the repository routing key (its full
/// `did:key`) plus its branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceRef {
    /// Repository routing key — the full `did:key`, reconstructed from
    /// the URL's trailing id. This is the repository's identity, not its
    /// display label.
    pub name: String,
    /// Branch name (the part before `@`, or [`DEFAULT_BRANCH`]).
    pub branch: String,
}

/// Parse a `{branch}@{label}:{id}` (or bare `{id}`) space segment.
///
/// A bare segment is the id on [`DEFAULT_BRANCH`]; a `branch@…` segment
/// pins the branch. Any `{label}:` prefix on the post-`@` remainder is a
/// display label and is dropped — the id is everything after the last
/// `:`. The routing key is `did:key:` + that id. An empty id yields
/// `None` — there is no repository to address.
pub fn parse_space(segment: &str) -> Option<SpaceRef> {
    let (branch, rest) = match segment.split_once('@') {
        Some((branch, rest)) => (branch.to_owned(), rest),
        None => (DEFAULT_BRANCH.to_owned(), segment),
    };
    // The id is the trailing component after the last `:`; any leading
    // `{label}:` is a cosmetic display label.
    let id = match rest.rsplit_once(':') {
        Some((_label, id)) => id,
        None => rest,
    };
    if id.is_empty() {
        return None;
    }
    Some(SpaceRef {
        name: format!("{DID_KEY_PREFIX}{id}"),
        branch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reconstructs_the_did_from_a_bare_id_on_the_default_branch() {
        assert_eq!(
            parse_space("z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[test]
    fn it_drops_a_label_prefix_and_reconstructs_the_did() {
        assert_eq!(
            parse_space("home:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[test]
    fn it_parses_a_branch_at_label_id() {
        assert_eq!(
            parse_space("feat@home:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "feat".into(),
            }),
        );
    }

    #[test]
    fn it_keeps_slashes_in_a_branch_name() {
        assert_eq!(
            parse_space("feat/artifact@home:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "feat/artifact".into(),
            }),
        );
    }

    #[test]
    fn it_takes_the_id_after_the_last_colon_of_a_full_did() {
        // A full `did:key:z…` segment resolves to the same DID — its id
        // is the trailing blob, which is reconstructed back to the DID.
        assert_eq!(
            parse_space("did:key:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[test]
    fn it_rejects_an_empty_id() {
        assert_eq!(parse_space(""), None);
        assert_eq!(parse_space("feat@"), None);
        assert_eq!(parse_space("home:"), None);
    }
}
