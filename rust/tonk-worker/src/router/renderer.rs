//! The renderer a space keeps: the files of the element runtime its page
//! loads, and the page itself, each an asset of the space that a
//! `tonk:route/asset` answers at its path. The page is at `/`, which a load
//! of any address the space's routes show is answered with.
//!
//! A space's kernel copies them from its deployment when the space is
//! created, so the space renders with the renderer it was created with,
//! wherever it syncs, until it takes up another.

use dialog_artifacts::{Asset, Entity, Value};
use dialog_query::{Output as _, Query, Term};
use tonk_schema::AssetRoute;
use tonk_schema::domain::route_asset::{Asset as RouteAsset, Path as RoutePath};

use super::blob::{MEDIA_TYPE, NAME};
use super::claim::RawClaim;
use super::repository::CONTENT_BRANCH;
use crate::TonkWorkerError;
use crate::worker::TonkState;

/// Where the page a space is loaded with is kept.
pub const PAGE: &str = "/";

/// The media type a file at `path` is served with, by its extension.
fn media_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map(|(_, extension)| extension) {
        Some("js" | "mjs") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("css") => "text/css",
        Some("json") => "application/json",
        _ if path == PAGE => "text/html; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

/// The route that answers `path` with the renderer's file there: one per
/// path, so a renderer taken up later replaces the one before it.
fn route_at(path: &str) -> Result<Entity, TonkWorkerError> {
    format!("route:{path}")
        .parse()
        .map_err(|error| TonkWorkerError::Internal(format!("the route at {path}: {error}")))
}

/// Keep `files`, each a path and the bytes served there, as the renderer of
/// the space `key`: each an asset of the space, under the media type its
/// extension names, and the route that answers its path with it. One commit,
/// so a page never finds half a renderer.
pub async fn keep(
    tonk: &TonkState,
    key: &str,
    files: Vec<(String, Vec<u8>)>,
) -> Result<(), TonkWorkerError> {
    let media: dialog_artifacts::Relation = MEDIA_TYPE
        .parse()
        .map_err(|error| TonkWorkerError::Internal(format!("bad attribute: {error}")))?;
    let named: dialog_artifacts::Relation = NAME
        .parse()
        .map_err(|error| TonkWorkerError::Internal(format!("bad attribute: {error}")))?;
    let mut transaction = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .transaction();
    for (path, bytes) in files {
        let asset = Asset::new(bytes);
        let entity = asset
            .entity()
            .map_err(|error| TonkWorkerError::Internal(format!("the asset at {path}: {error}")))?;
        let name = path.rsplit('/').next().unwrap_or_default();
        let name = if name.is_empty() { "index.html" } else { name };
        transaction = transaction
            .assert(asset)
            .assert(RawClaim {
                the: media.clone(),
                of: entity.clone(),
                is: Value::String(media_type(&path).to_owned()),
                policy: dialog_artifacts::Pick::Last,
            })
            .assert(RawClaim {
                the: named.clone(),
                of: entity.clone(),
                is: Value::String(name.to_owned()),
                policy: dialog_artifacts::Pick::Last,
            })
            .assert(AssetRoute {
                this: route_at(&path)?,
                path: RoutePath(path),
                asset: RouteAsset(entity),
            });
    }
    transaction
        .commit()
        .perform(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("the renderer of {key}: {error}")))?;
    tonk.reactor.run_scheduled_polls(&tonk.operator).await;
    Ok(())
}

/// Whether the space `key`'s `main` holds any content.
pub async fn holds(tonk: &TonkState, key: &str) -> Result<bool, TonkWorkerError> {
    let main = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{key} main: {error}")))?;
    Ok(main.handle().revision().is_some())
}

/// Whether the space `key` keeps a page to be loaded with.
pub async fn kept(tonk: &TonkState, key: &str) -> Result<bool, TonkWorkerError> {
    let session = tonk
        .reactor
        .repository(key)
        .branch(CONTENT_BRANCH)
        .acquire(&tonk.operator)
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{key} main: {error}")))?;
    let routes = session
        .handle()
        .query()
        .select(Query::<AssetRoute> {
            this: Term::var("this"),
            path: Term::from(PAGE.to_owned()),
            asset: Term::var("asset"),
        })
        .perform(&tonk.operator)
        .try_vec()
        .await
        .map_err(|error| TonkWorkerError::Internal(format!("{key}'s routes: {error}")))?;
    Ok(!routes.is_empty())
}

#[cfg(test)]
mod tests {
    use super::super::repository::profile_library_tests::test_state;
    use super::super::repository::{
        BranchConfiguration, RepositoryConfiguration, create_repository,
    };
    use super::super::session::{Matched, match_route, match_view};
    use super::*;

    /// A space with a content branch and nothing on it.
    async fn space(tonk: &TonkState) -> String {
        let repository = create_repository(
            tonk,
            "Rendered",
            &RepositoryConfiguration::default()
                .branch(CONTENT_BRANCH, BranchConfiguration::default()),
        )
        .await
        .expect("the space creates");
        tonk_schema::prelude::DidExt::repo_key(&repository.did()).to_owned()
    }

    fn renderer(page: &str) -> Vec<(String, Vec<u8>)> {
        vec![
            (PAGE.to_owned(), page.as_bytes().to_vec()),
            ("/guest/manifest.json".to_owned(), b"{}".to_vec()),
            ("/guest/guest_bg.wasm".to_owned(), vec![0, 97, 115, 109]),
        ]
    }

    async fn content(tonk: &TonkState, key: &str) -> crate::reactor::BranchSession {
        tonk.reactor
            .repository(key)
            .branch(CONTENT_BRANCH)
            .acquire(&tonk.operator)
            .await
            .expect("the content branch opens")
    }

    #[dialog_common::test]
    async fn it_answers_each_file_of_the_renderer_at_its_path() {
        let tonk = test_state().await;
        let key = space(&tonk).await;
        assert!(!kept(&tonk, &key).await.unwrap());

        keep(&tonk, &key, renderer("<!doctype html>"))
            .await
            .unwrap();

        assert!(kept(&tonk, &key).await.unwrap());
        let session = content(&tonk, &key).await;
        let Some(Matched::Asset(wasm)) = match_route(&tonk, &session, "/guest/guest_bg.wasm").await
        else {
            panic!("the wasm is answered with an asset");
        };
        assert_eq!(
            wasm.asset,
            Asset::new(vec![0, 97, 115, 109]).entity().unwrap()
        );
        assert!(matches!(
            match_route(&tonk, &session, "/").await,
            Some(Matched::Asset(_))
        ));
    }

    #[dialog_common::test]
    async fn it_replaces_the_renderer_a_space_kept_before() {
        let tonk = test_state().await;
        let key = space(&tonk).await;

        keep(&tonk, &key, renderer("first")).await.unwrap();
        keep(&tonk, &key, renderer("second")).await.unwrap();

        let pages = content(&tonk, &key)
            .await
            .handle()
            .query()
            .select(Query::<AssetRoute> {
                this: Term::var("this"),
                path: Term::from(PAGE.to_owned()),
                asset: Term::var("asset"),
            })
            .perform(&tonk.operator)
            .try_vec()
            .await
            .unwrap();
        assert_eq!(pages.len(), 1);
        assert_eq!(
            pages[0].asset.0,
            Asset::new(b"second".to_vec()).entity().unwrap()
        );
    }

    #[dialog_common::test]
    async fn it_leaves_the_page_out_of_what_a_page_shows() {
        let tonk = test_state().await;
        let key = space(&tonk).await;
        keep(&tonk, &key, renderer("<!doctype html>"))
            .await
            .unwrap();

        let session = content(&tonk, &key).await;
        assert!(match_view(&tonk, &session, "/").await.is_none());
    }

    #[dialog_common::test]
    async fn it_serves_each_file_under_the_media_type_of_its_extension() {
        assert_eq!(media_type("/"), "text/html; charset=utf-8");
        assert_eq!(media_type("/guest/guest-1.js"), "text/javascript");
        assert_eq!(media_type("/guest/snippets/a/b.mjs"), "text/javascript");
        assert_eq!(media_type("/guest/guest_bg-1.wasm"), "application/wasm");
        assert_eq!(media_type("/styles-1.css"), "text/css");
        assert_eq!(media_type("/guest/manifest.json"), "application/json");
    }
}
