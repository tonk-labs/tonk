//! Shared helpers for building blob route URLs from a `<tonk-display>`/
//! `<tonk-upload>` `with="{branch}@{repo}"` context attribute.

/// Parse a `with="{branch}@{repo}"` context into `(branch, repo)`. Returns
/// `None` if `with` is empty or still an unsubstituted `{…}` template. A bare
/// token with no `@` is a repo on the default branch `main`.
pub(crate) fn branch_repo(with: &str) -> Option<(String, String)> {
    if with.is_empty() || with.contains('{') {
        return None;
    }
    // `with` is `branch@repo`; a bare token (no `@`) is a repo on `main`.
    let (branch, repo) = match with.split_once('@') {
        Some((branch, repo)) => (branch, repo),
        None => ("main", with),
    };
    if repo.is_empty() {
        return None;
    }
    Some((branch.to_string(), repo.to_string()))
}

/// The read URL for a blob entity, scoped by `with`. `None` if `with` is unusable.
pub(crate) fn blob_read_url(with: &str, entity: &str) -> Option<String> {
    let (branch, repo) = branch_repo(with)?;
    Some(format!(
        "/api/repository/{repo}/branch/{branch}/blob/{entity}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    #[dialog_common::test]
    fn it_parses_branch_and_repo() {
        assert_eq!(
            branch_repo("main@did:key:zX"),
            Some(("main".into(), "did:key:zX".into()))
        );
        assert_eq!(
            branch_repo("did:key:zX"),
            Some(("main".into(), "did:key:zX".into()))
        );
        assert_eq!(branch_repo(""), None);
        assert_eq!(branch_repo("{branch}@{repo}"), None);
    }

    #[dialog_common::test]
    fn it_builds_the_read_url() {
        assert_eq!(
            blob_read_url("main@repo", "blob:zH").as_deref(),
            Some("/api/repository/repo/branch/main/blob/blob:zH"),
        );
        assert_eq!(blob_read_url("{x}", "blob:zH"), None);
    }
}
