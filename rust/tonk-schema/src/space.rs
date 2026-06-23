//! Level 0 routing: resolve a URL's space segment to a `(repository, branch)`
//! target.
//!
//! The SW uses this as its Level 0 builtin — the containment boundary, code not
//! data, so no overlay fact can subvert which database a request may touch — and
//! the UI uses it to parse the same segment out of the location. Every route's
//! space segment is a single `:space` param that encodes a repository's id, an
//! optional human-readable label, and an optional branch:
//!
//! - `z6MkABC`            → key `did:key:z6MkABC`, branch `main`
//! - `home:z6MkABC`       → key `did:key:z6MkABC`, branch `main`
//! - `feat@home:z6MkABC`  → key `did:key:z6MkABC`, branch `feat`
//!
//! A repository's identity is its credential's `did:key`. The URL carries only
//! the trailing id (the multibase blob after `did:key:`) plus an optional
//! cosmetic `{label}:` prefix, to keep URLs short; the routing key reconstructs
//! the full `did:key:` + id, which is what names the database, the reactor
//! cache, and the `<tonk-repository>` routing context. `@` separates an optional
//! leading branch; within the rest, the id is everything after the LAST `:` (a
//! bare segment with no `:` is itself the id). Branch names may contain `:` and
//! `/` but not `@`, so the branch split is unambiguous.

/// The default branch when the space segment names none.
pub const DEFAULT_BRANCH: &str = "main";

/// The `did:key:` prefix the URL id reconstructs into the full DID.
const DID_KEY_PREFIX: &str = "did:key:";

/// A parsed space segment: the repository routing key (its full `did:key`) plus
/// its branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceRef {
    /// Repository routing key — the full `did:key`, reconstructed from the URL's
    /// trailing id. This is the repository's identity, not its display label.
    pub name: String,
    /// Branch name (the part before `@`, or [`DEFAULT_BRANCH`]).
    pub branch: String,
}

/// Parse a `{branch}@{label}:{id}` (or bare `{id}`) space segment.
///
/// A bare segment is the id on [`DEFAULT_BRANCH`]; a `branch@…` segment pins the
/// branch. Any `{label}:` prefix on the post-`@` remainder is a display label
/// and is dropped — the id is everything after the last `:`. The routing key is
/// `did:key:` + that id. An empty id yields `None` — there is no repository to
/// address.
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

/// The logical target a document path resolves to — Level 0's verdict on which
/// database a request may touch. The caller maps it to a concrete branch (a
/// named repository for [`Self::Space`], the profile's meta branch for
/// [`Self::Profile`]); resolving the target is pure, mapping it is the SW's job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RouteTarget {
    /// A named space: `/space/{segment}/…`.
    Space(SpaceRef),
    /// The profile (its meta branch): `/` and `/join`.
    Profile,
}

/// Resolve a document path to its Level 0 [`RouteTarget`].
///
/// `/space/{segment}/…` resolves the segment via [`parse_space`]; `/` and
/// `/join` are the profile. An unknown prefix or an unparseable space segment
/// yields `None` — there is no database to address. A leading scheme/host is not
/// expected; pass the pathname (what `location.pathname` gives, and what the
/// SW extracts from `Referer`).
pub fn resolve_path(path: &str) -> Option<RouteTarget> {
    let path = path.trim_start_matches('/');
    let (head, rest) = match path.split_once('/') {
        Some((head, rest)) => (head, rest),
        None => (path, ""),
    };
    match head {
        "" | "join" => Some(RouteTarget::Profile),
        "space" => {
            // The space segment is the first component after `space/`; anything
            // past it is the remaining (Level 1) path, ignored here.
            let segment = rest.split_once('/').map(|(seg, _)| seg).unwrap_or(rest);
            parse_space(segment).map(RouteTarget::Space)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test_configure;

    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    async fn it_reconstructs_the_did_from_a_bare_id_on_the_default_branch() {
        assert_eq!(
            parse_space("z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[dialog_common::test]
    async fn it_drops_a_label_prefix_and_reconstructs_the_did() {
        assert_eq!(
            parse_space("home:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[dialog_common::test]
    async fn it_parses_a_branch_at_label_id() {
        assert_eq!(
            parse_space("feat@home:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "feat".into(),
            }),
        );
    }

    #[dialog_common::test]
    async fn it_keeps_slashes_in_a_branch_name() {
        assert_eq!(
            parse_space("feat/artifact@home:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "feat/artifact".into(),
            }),
        );
    }

    #[dialog_common::test]
    async fn it_takes_the_id_after_the_last_colon_of_a_full_did() {
        // A full `did:key:z…` segment resolves to the same DID — its id is the
        // trailing blob, which is reconstructed back to the DID.
        assert_eq!(
            parse_space("did:key:z6MkABC"),
            Some(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "main".into(),
            }),
        );
    }

    #[dialog_common::test]
    async fn it_rejects_an_empty_id() {
        assert_eq!(parse_space(""), None);
        assert_eq!(parse_space("feat@"), None);
        assert_eq!(parse_space("home:"), None);
    }

    #[dialog_common::test]
    async fn it_resolves_root_and_join_to_the_profile() {
        assert_eq!(resolve_path("/"), Some(RouteTarget::Profile));
        assert_eq!(resolve_path("/join"), Some(RouteTarget::Profile));
        assert_eq!(resolve_path(""), Some(RouteTarget::Profile));
    }

    #[dialog_common::test]
    async fn it_resolves_a_space_path_to_its_target() {
        assert_eq!(
            resolve_path("/space/home:z6MkABC"),
            Some(RouteTarget::Space(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "main".into(),
            })),
        );
    }

    #[dialog_common::test]
    async fn it_ignores_the_remaining_path_after_the_space_segment() {
        // `/space/{seg}/board` resolves the same target as `/space/{seg}` —
        // the remaining path is Level 1's concern, not Level 0's.
        assert_eq!(
            resolve_path("/space/feat@home:z6MkABC/board/extra"),
            Some(RouteTarget::Space(SpaceRef {
                name: "did:key:z6MkABC".into(),
                branch: "feat".into(),
            })),
        );
    }

    #[dialog_common::test]
    async fn it_rejects_an_unknown_prefix_or_empty_space_segment() {
        assert_eq!(resolve_path("/nope"), None);
        assert_eq!(resolve_path("/space/"), None);
        assert_eq!(resolve_path("/space"), None);
    }
}
