//! The document sync pass: keep the local cell and the remote cell
//! merged.
//!
//! The pass is the loop in Irakli's note — resolve the remote, merge,
//! publish with compare-and-swap, retry when the swap loses — and it is
//! shared code: the worker runs it in its sync drain, the CLI in its
//! auto-sync.
//!
//! No change is ever lost. A publish names the remote version this
//! replica last merged, so whatever it replaces is already contained in
//! what it writes and the remote only grows. Merge is commutative and
//! idempotent, so any order of passes reaches the same document. And
//! after every resolve the pass compares HEADS, not only the version
//! token: a faulty writer that overwrote the cell with older bytes is
//! healed by the next replica that holds more.

use dialog_effects::memory::Version;
use serde::{Deserialize, Serialize};

use crate::cell::{CellError, RETRY_LIMIT, Transport, save};
use crate::engine::Document;

/// What this replica last knew of the remote cell. Local and remote
/// version tokens are different kinds (a content hash against an ETag),
/// so the remote's needs its own record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marker {
    /// The remote version last merged, hex. `None` = the remote was empty.
    pub version: Option<String>,
    /// The store heads the remote held at that version.
    pub heads: Vec<String>,
}

impl Marker {
    fn remote_version(&self) -> Option<Version> {
        self.version
            .as_deref()
            .and_then(|text| hex::decode(text).ok())
            .map(Version::from)
    }

    fn at(version: &Version, heads: Vec<String>) -> Self {
        Self {
            version: Some(hex::encode(version.as_bytes())),
            heads,
        }
    }
}

/// What a pass did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// New changes arrived from the remote; the mirror is stale.
    pub pulled: bool,
    /// Local changes were published to the remote.
    pub pushed: bool,
}

async fn read_marker<M: Transport + ?Sized>(marker: &M) -> Result<(Marker, Option<Version>), CellError> {
    Ok(match marker.resolve().await? {
        Some((bytes, version)) => (serde_json::from_slice(&bytes).unwrap_or_default(), Some(version)),
        None => (Marker::default(), None),
    })
}

/// One pass over one document. `marker` is a small local cell of its
/// own (`remote/<remote>/document/<id>` / `synced`).
pub async fn sync_pass<L, R, M>(local: &L, remote: &R, marker: &M) -> Result<Outcome, CellError>
where
    L: Transport + ?Sized,
    R: Transport + ?Sized,
    M: Transport + ?Sized,
{
    let mut outcome = Outcome::default();
    for _ in 0..RETRY_LIMIT {
        let (known, marker_version) = read_marker(marker).await?;
        let mine = local.resolve().await?;
        let theirs = remote.resolve().await?;

        let (mut document, local_version) = match (&mine, &theirs) {
            (None, None) => return Ok(outcome),
            (Some((bytes, version)), _) => (Document::load(bytes)?, Some(version.clone())),
            (None, Some((bytes, _))) => (Document::load(bytes)?, None),
        };
        let before = document.store_heads();
        let mut grew = mine.is_none();

        // Compare heads, not only the version: see the module docs.
        let remote_heads = match &theirs {
            Some((bytes, version)) if known.remote_version().as_ref() != Some(version) => {
                grew |= document.merge(bytes)?;
                Document::load(bytes)?.store_heads()
            }
            Some(_) => known.heads.clone(),
            None => Vec::new(),
        };

        let ours = document.store_heads();
        let remote_version = theirs.as_ref().map(|(_, version)| version.clone());
        let published = if ours != remote_heads {
            match remote.publish(document.save(), remote_version.clone()).await {
                Ok(version) => Some(version),
                Err(CellError::Conflict) => continue,
                Err(other) => return Err(other),
            }
        } else {
            None
        };

        if grew || ours != before {
            save(local, &mut document, local_version).await?;
            outcome.pulled = true;
        }
        if published.is_some() {
            outcome.pushed = true;
        }
        let settled = published.or(remote_version);
        if let Some(version) = settled {
            let next = Marker::at(&version, document.store_heads());
            if next != known {
                // Best effort: a lost marker only costs one extra merge.
                let bytes = serde_json::to_vec(&next).unwrap_or_default();
                let _ = marker.publish(bytes, marker_version).await;
            }
        }
        return Ok(outcome);
    }
    Err(CellError::Contended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::tests::MemoryCell;
    use crate::cell::load;
    use crate::engine::{Edit, Stamp};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test_configure;
    #[cfg(target_arch = "wasm32")]
    wasm_bindgen_test_configure!(run_in_browser);

    struct Replica {
        local: MemoryCell,
        marker: MemoryCell,
    }

    impl Replica {
        fn new() -> Self {
            Self {
                local: MemoryCell::default(),
                marker: MemoryCell::default(),
            }
        }

        async fn write(&self, text: &str) -> Vec<String> {
            let (mut document, version) = match load(&self.local).await.unwrap() {
                Some((document, version)) => (document, Some(version)),
                None => (Document::from_text("").unwrap(), None),
            };
            let heads = document.store_heads();
            let after = document
                .edit(&heads, &Stamp::default(), &Edit::Splice { at: 0, delete: 0, text: text.into() })
                .unwrap();
            save(&self.local, &mut document, version).await.unwrap();
            after
        }

        async fn text(&self) -> String {
            let (mut document, _) = load(&self.local).await.unwrap().unwrap();
            let heads = document.store_heads();
            document.text(&heads).unwrap()
        }

        async fn sync(&self, remote: &MemoryCell) -> Outcome {
            sync_pass(&self.local, remote, &self.marker).await.unwrap()
        }
    }

    #[dialog_common::test]
    async fn it_converges_two_replicas_through_one_remote_cell() {
        let remote = MemoryCell::default();
        let a = Replica::new();
        let b = Replica::new();
        a.write("from a. ").await;
        b.write("from b. ").await;

        assert!(a.sync(&remote).await.pushed, "a creates the remote cell");
        let second = b.sync(&remote).await;
        assert!(second.pulled && second.pushed, "b merges a's changes and publishes the union");
        assert!(a.sync(&remote).await.pulled);

        let (ta, tb) = (a.text().await, b.text().await);
        assert_eq!(ta, tb);
        assert!(ta.contains("from a.") && ta.contains("from b."), "{ta:?}");

        let idle = a.sync(&remote).await;
        assert_eq!(idle, Outcome::default(), "a settled pass moves nothing");
    }

    #[dialog_common::test]
    async fn it_restores_the_union_after_a_faulty_overwrite() {
        let remote = MemoryCell::default();
        let a = Replica::new();
        a.write("first. ").await;
        a.sync(&remote).await;
        let older = remote.resolve().await.unwrap().unwrap().0;

        a.write("second. ").await;
        a.sync(&remote).await;

        // A faulty writer puts the older bytes back.
        remote.clobber(older);
        let healed = a.sync(&remote).await;
        assert!(healed.pushed, "the replica that holds more publishes the union");
        let (mut document, _) = load(&remote).await.unwrap().unwrap();
        let heads = document.store_heads();
        assert!(document.text(&heads).unwrap().contains("second."));
    }

    #[dialog_common::test]
    async fn it_opens_a_document_that_exists_only_on_the_remote() {
        let remote = MemoryCell::default();
        let a = Replica::new();
        a.write("hello").await;
        a.sync(&remote).await;

        let fresh = Replica::new();
        let outcome = fresh.sync(&remote).await;
        assert!(outcome.pulled && !outcome.pushed);
        assert_eq!(fresh.text().await, "hello");
    }
}
