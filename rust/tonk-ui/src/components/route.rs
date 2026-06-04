//! Shared parsing for the `{branch}@{name}` space segment.
//!
//! Every route's space segment is a single `:space` param that encodes
//! both the repository name and (optionally) a branch:
//!
//! - `home`        → name `home`, branch `main` (the default)
//! - `feat@home`   → name `home`, branch `feat`
//!
//! `@` separates an optional leading branch from the required name.
//! Repository names and branch names may contain `:` and `/` but not
//! `@`, so the split is unambiguous. Centralizing the parse keeps the
//! convention identical across the display, concept, board, layout, and
//! space-viewer routes.

/// The default branch when the space segment names none.
pub const DEFAULT_BRANCH: &str = "main";

/// A parsed space segment: the repository name plus its branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpaceRef {
    /// Repository name (the part after `@`, or the whole segment).
    pub name: String,
    /// Branch name (the part before `@`, or [`DEFAULT_BRANCH`]).
    pub branch: String,
}

/// Parse a `{branch}@{name}` (or bare `{name}`) space segment.
///
/// A bare segment is the name on [`DEFAULT_BRANCH`]; a `branch@name`
/// segment pins both. An empty name yields `None` — there is no
/// repository to address.
pub fn parse_space(segment: &str) -> Option<SpaceRef> {
    let (branch, name) = match segment.split_once('@') {
        Some((branch, name)) => (branch.to_owned(), name.to_owned()),
        None => (DEFAULT_BRANCH.to_owned(), segment.to_owned()),
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
    fn it_parses_a_bare_name_on_the_default_branch() {
        assert_eq!(
            parse_space("home"),
            Some(SpaceRef {
                name: "home".into(),
                branch: "main".into(),
            }),
        );
    }

    #[test]
    fn it_parses_a_branch_at_name() {
        assert_eq!(
            parse_space("feat@home"),
            Some(SpaceRef {
                name: "home".into(),
                branch: "feat".into(),
            }),
        );
    }

    #[test]
    fn it_keeps_slashes_in_a_branch_name() {
        assert_eq!(
            parse_space("feat/artifact@home"),
            Some(SpaceRef {
                name: "home".into(),
                branch: "feat/artifact".into(),
            }),
        );
    }

    #[test]
    fn it_rejects_an_empty_name() {
        assert_eq!(parse_space(""), None);
        assert_eq!(parse_space("feat@"), None);
    }
}
