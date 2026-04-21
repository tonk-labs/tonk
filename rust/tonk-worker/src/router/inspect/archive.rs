//! Archive block inspection routes.

use ::axum::extract::Path;
use ::axum::{Json, extract::State};
use axum_wasm_macros::wasm_compat;
use base58::FromBase58;
use dialog_effects::archive as archive_fx;
use dialog_repository::{RepositoryArchiveExt as _, RepositoryExt as _};
use dialog_storage::Blake3Hash;
use serde::{Deserialize, Serialize};
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
use tokio::sync::oneshot;
use tonk_common::log;

use super::super::AppState;
use crate::TonkWorkerError;

/// Path parameters for local archive block fetch.
#[derive(Debug, Deserialize)]
pub struct ArchiveBlockPath {
    /// The repository name.
    pub repo: String,
    /// The block hash (base58).
    pub hash: String,
}

/// Path parameters for remote archive block fetch.
#[derive(Debug, Deserialize)]
pub struct RemoteArchiveBlockPath {
    /// The repository name.
    pub repo: String,
    /// The remote name.
    pub remote: String,
    /// The block hash (base58).
    pub hash: String,
}

/// Response for archive block fetch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArchiveBlockResponse {
    /// The requested hash (base58).
    pub hash: String,
    /// Whether the block was found.
    pub found: bool,
    /// The block data as base64 (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// The block size in bytes (if found).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<usize>,
    /// Error message if fetch failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Decode a base58 hash string into a Blake3Hash.
fn decode_hash(hash_str: &str) -> Result<Blake3Hash, String> {
    let hash_bytes = hash_str
        .from_base58()
        .map_err(|e| format!("Invalid base58 hash: {:?}", e))?;

    let hash_array: [u8; 32] = hash_bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| format!("Hash must be 32 bytes, got {}", bytes.len()))?;

    Ok(Blake3Hash::from(hash_array))
}

/// Fetches a block from the local archive by its blake3 hash.
#[wasm_compat]
pub async fn inspect_archive_block(
    State(state): State<AppState>,
    Path(params): Path<ArchiveBlockPath>,
) -> Result<Json<ArchiveBlockResponse>, TonkWorkerError> {
    log!(
        "Fetching local archive block: repo={}, hash={}",
        params.repo,
        params.hash
    );

    let hash = match decode_hash(&params.hash) {
        Ok(h) => h,
        Err(e) => {
            return Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(e),
            }));
        }
    };

    let tonk_state = state.read().await;

    let repo = match tonk_state
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("Failed to load repo: {}", e)),
            }));
        }
    };

    // Load the main branch and read from its archive index catalog
    let branch = match repo
        .branch("main")
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(b) => b,
        Err(e) => {
            return Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("Failed to load branch: {}", e)),
            }));
        }
    };
    let catalog = branch.archive().index();
    let get = archive_fx::Get::new(hash);
    let effect = catalog.invoke(get);

    match effect.perform(&tonk_state.operator).await {
        Ok(Some(data)) => {
            use base64::Engine;
            let data: Vec<u8> = data;
            let size = data.len();
            let data_base64 = base64::engine::general_purpose::STANDARD.encode(&data);
            Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: true,
                data: Some(data_base64),
                size: Some(size),
                error: None,
            }))
        }
        Ok(None) => Ok(Json(ArchiveBlockResponse {
            hash: params.hash,
            found: false,
            data: None,
            size: None,
            error: None,
        })),
        Err(e) => {
            log!("Error fetching archive block: {:?}", e);
            Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("{}", e)),
            }))
        }
    }
}

/// Fetches a block from a remote's archive by its blake3 hash.
///
/// Uses `RemoteRepository::archive().index().get(hash)` to read directly
/// from the remote site.
#[wasm_compat]
pub async fn inspect_remote_archive_block(
    State(state): State<AppState>,
    Path(params): Path<RemoteArchiveBlockPath>,
) -> Result<Json<ArchiveBlockResponse>, TonkWorkerError> {
    log!(
        "Fetching remote archive block: repo={}, remote={}, hash={}",
        params.repo,
        params.remote,
        params.hash
    );

    let hash = match decode_hash(&params.hash) {
        Ok(h) => h,
        Err(e) => {
            return Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(e),
            }));
        }
    };

    let tonk_state = state.read().await;

    let repo = match tonk_state
        .profile
        .repository(&params.repo)
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("Failed to load repo: {}", e)),
            }));
        }
    };

    let remote_repo = match repo
        .remote(params.remote.as_str())
        .load()
        .perform(&tonk_state.operator)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            return Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("Remote '{}' not found: {}", params.remote, e)),
            }));
        }
    };

    // Read the block directly from the remote's archive index
    let result = remote_repo
        .archive()
        .index()
        .get(hash)
        .perform(&tonk_state.operator)
        .await;

    match result {
        Ok(Some(data)) => {
            use base64::Engine;
            let data: Vec<u8> = data;
            let size = data.len();
            let data_base64 = base64::engine::general_purpose::STANDARD.encode(&data);
            Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: true,
                data: Some(data_base64),
                size: Some(size),
                error: None,
            }))
        }
        Ok(None) => Ok(Json(ArchiveBlockResponse {
            hash: params.hash,
            found: false,
            data: None,
            size: None,
            error: None,
        })),
        Err(e) => {
            log!("Error fetching remote archive block: {:?}", e);
            Ok(Json(ArchiveBlockResponse {
                hash: params.hash,
                found: false,
                data: None,
                size: None,
                error: Some(format!("{}", e)),
            }))
        }
    }
}
