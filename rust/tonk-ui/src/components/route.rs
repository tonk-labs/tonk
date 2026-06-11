//! Shared parsing for the `{branch}@{label}:{key}` space segment.
//!
//! Every route's space segment is a single `:space` param that encodes a
//! repository's routing key, an optional human-readable label, and an
//! optional branch:
//!
//! - `z6MkABC`            → key `z6MkABC`, branch `main` (the default)
//! - `home:z6MkABC`       → key `z6MkABC`, branch `main`
//! - `feat@home:z6MkABC`  → key `z6MkABC`, branch `feat`
//!
//! A repository's identity is its credential's `did:key`; the routing
//! key is the DID suffix (the part after the last `:`). The optional
//! `{label}:` prefix is a display name only and is ignored when routing.
//!
//! `@` separates an optional leading branch from the rest; within the
//! rest, the key is everything after the LAST `:` (a bare segment with
//! no `:` is itself the key). Branch names may contain `:` and `/` but
//! not `@`, so the branch split is unambiguous, and the key is taken
//! after the last `:` so a `did:key:z…` label segment still resolves to
//! the trailing key. Centralizing the parse keeps the convention
//! identical across the display, concept, board, layout, and
//! space-viewer routes.

/// The default branch when the space segment names none.
pub const DEFAULT_BRANCH: &str = "main";

/// A parsed space segment: the repository routing key plus its branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceRef {
    /// Repository routing key — the part after the last `:` in the
    /// post-`@` remainder (or the whole remainder when it has no `:`).
    /// This is the repository's identity, not its display label.
    pub name: String,
    /// Branch name (the part before `@`, or [`DEFAULT_BRANCH`]).
    pub branch: String,
}

/// Parse a `{branch}@{label}:{key}` (or bare `{key}`) space segment.
///
/// A bare segment is the key on [`DEFAULT_BRANCH`]; a `branch@…` segment
/// pins the branch. Any `{label}:` prefix on the post-`@` remainder is a
/// display label and is dropped — the routing key is everything after
/// the last `:`. An empty key yields `None` — there is no repository to
/// address.
pub fn parse_space(segment: &str) -> Option<SpaceRef> {
    let (branch, rest) = match segment.split_once('@') {
        Some((branch, rest)) => (branch.to_owned(), rest),
        None => (DEFAULT_BRANCH.to_owned(), segment),
    };
    // The routing key is the trailing component after the last `:`; any
    // leading `{label}:` (or `did:key:` prefix) is a display label only.
    let name = match rest.rsplit_once(':') {
        Some((_label, key)) => key.to_owned(),
        None => rest.to_owned(),
    };
    if name.is_empty() {
        return None;
    }
    Some(SpaceRef { name, branch })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_a_bare_key_on_the_default_branch() {
        assert_eq!(
            parse_space("z6MkABC"),
            Some(SpaceRef {
                name: "z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[test]
    fn it_drops_a_label_prefix_and_keeps_the_key() {
        assert_eq!(
            parse_space("home:z6MkABC"),
            Some(SpaceRef {
                name: "z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[test]
    fn it_parses_a_branch_at_label_key() {
        assert_eq!(
            parse_space("feat@home:z6MkABC"),
            Some(SpaceRef {
                name: "z6MkABC".into(),
                branch: "feat".into(),
            }),
        );
    }

    #[test]
    fn it_keeps_slashes_in_a_branch_name() {
        assert_eq!(
            parse_space("feat/artifact@home:z6MkABC"),
            Some(SpaceRef {
                name: "z6MkABC".into(),
                branch: "feat/artifact".into(),
            }),
        );
    }

    #[test]
    fn it_takes_the_key_after_the_last_colon_of_a_did() {
        // A full `did:key:z…` label resolves to its trailing key.
        assert_eq!(
            parse_space("did:key:z6MkABC"),
            Some(SpaceRef {
                name: "z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[test]
    fn it_rejects_an_empty_key() {
        assert_eq!(parse_space(""), None);
        assert_eq!(parse_space("feat@"), None);
        assert_eq!(parse_space("home:"), None);
    }
}
