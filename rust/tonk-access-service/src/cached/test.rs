//! The policy against the in-memory store and an in-memory cache.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dialog_capability::{Provider, Subject, did};
use dialog_common::{Blake3Hash, Buffer};
use dialog_effects::MethodExt as _;
use dialog_effects::archive::prelude::*;
use dialog_effects::blob::prelude::*;
use dialog_effects::blob::{BlobError, BlobReader, BlobSource, ByteRange, Read};
use dialog_effects::memory::prelude::*;
use dialog_remote_ucan::helpers::MemoryStore;

use super::{Cached, Mode, ObjectCache, Outcome, blob_key, block_key, collect};

/// A cache that holds objects in a map.
#[derive(Clone, Default)]
struct MemoryCache(Arc<Mutex<HashMap<String, Vec<u8>>>>);

impl MemoryCache {
    fn holds(&self, key: &str) -> bool {
        self.0.lock().unwrap().contains_key(key)
    }
}

/// A source over one buffer.
struct Whole(Option<Vec<u8>>);

#[async_trait::async_trait]
impl BlobSource for Whole {
    async fn next(&mut self) -> Result<Option<Vec<u8>>, BlobError> {
        Ok(self.0.take())
    }
}

#[async_trait::async_trait]
impl ObjectCache for MemoryCache {
    async fn read(&self, key: &str, range: Option<ByteRange>) -> Option<BlobReader> {
        let bytes = self.0.lock().unwrap().get(key).cloned()?;
        let bytes = match range {
            Some(range) => {
                let start = (range.offset as usize).min(bytes.len());
                let end = range.length.map_or(bytes.len(), |length| {
                    (start + length as usize).min(bytes.len())
                });
                bytes[start..end].to_vec()
            }
            None => bytes,
        };
        Some(Box::new(Whole(Some(bytes))))
    }

    async fn write(&self, key: &str, bytes: Vec<u8>) {
        self.0.lock().unwrap().insert(key.to_string(), bytes);
    }
}

fn subject() -> Subject {
    Subject::from(did!("key:z6MkhaXgBZDvotDkL5257faiztiGiC2QtKLGpbnnEGta2doK"))
}

/// Run the fills an operation left behind, as the embedder does once
/// the response is on its way.
async fn settle<P, C>(cached: &Cached<P, C>) {
    for fill in cached.take_fills() {
        fill.await;
    }
}

#[tokio::test]
async fn a_block_written_is_read_from_the_cache_thereafter() {
    let cache = MemoryCache::default();
    let content = b"a block".to_vec();
    let digest = Blake3Hash::hash(&content);

    let writer = Cached::new(MemoryStore::default(), cache.clone());
    Provider::<dialog_effects::archive::Put>::execute(
        &writer,
        subject()
            .writer()
            .archive()
            .catalog("index")
            .put(Buffer::from(content.clone())),
    )
    .await
    .unwrap();
    assert_eq!(writer.outcome(), Outcome::Fill);
    settle(&writer).await;

    // A store that never saw the block, behind the same cache.
    let reader = Cached::new(MemoryStore::default(), cache.clone());
    let served = Provider::<dialog_effects::archive::Get>::execute(
        &reader,
        subject().reader().archive().catalog("index").get(digest),
    )
    .await
    .unwrap();
    assert_eq!(served, Some(content));
    assert_eq!(reader.outcome(), Outcome::Hit);
}

#[tokio::test]
async fn a_missed_block_is_served_by_the_store_and_fills_the_cache() {
    let cache = MemoryCache::default();
    let store = MemoryStore::default();
    let content = b"a block the store holds".to_vec();
    let digest = Blake3Hash::hash(&content);
    let put = subject()
        .writer()
        .archive()
        .catalog("index")
        .put(Buffer::from(content.clone()));
    let key = block_key(put.subject(), put.catalog(), &digest);
    Provider::<dialog_effects::archive::Put>::execute(&store, put)
        .await
        .unwrap();

    let cached = Cached::new(store, cache.clone());
    let get = subject().reader().archive().catalog("index").get(digest);
    let served = Provider::<dialog_effects::archive::Get>::execute(&cached, get)
        .await
        .unwrap();
    assert_eq!(served, Some(content));
    assert_eq!(cached.outcome(), Outcome::Miss);
    assert!(!cache.holds(&key), "the fill runs behind the answer");
    settle(&cached).await;
    assert!(cache.holds(&key));
}

#[tokio::test]
async fn a_block_nobody_holds_is_a_miss_that_fills_nothing() {
    let cache = MemoryCache::default();
    let cached = Cached::new(MemoryStore::default(), cache.clone());
    let served = Provider::<dialog_effects::archive::Get>::execute(
        &cached,
        subject()
            .reader()
            .archive()
            .catalog("index")
            .get(Blake3Hash::hash(b"never")),
    )
    .await
    .unwrap();
    assert_eq!(served, None);
    assert_eq!(cached.outcome(), Outcome::Miss);
    assert!(cached.take_fills().is_empty());
}

#[tokio::test]
async fn bypass_reads_the_store_past_a_cache_that_holds_the_block() {
    let cache = MemoryCache::default();
    let content = b"cached but bypassed".to_vec();
    let digest = Blake3Hash::hash(&content);
    let get = subject()
        .reader()
        .archive()
        .catalog("index")
        .get(digest.clone());
    cache
        .write(&block_key(get.subject(), get.catalog(), &digest), content)
        .await;

    let cached = Cached::new(MemoryStore::default(), cache.clone()).with_mode(Mode::Bypass);
    let served = Provider::<dialog_effects::archive::Get>::execute(&cached, get)
        .await
        .unwrap();
    assert_eq!(served, None, "the empty store answered, not the cache");
    assert_eq!(cached.outcome(), Outcome::Bypass);
}

fn blob() -> Vec<u8> {
    (0..5_000u32).flat_map(|i| i.to_le_bytes()).collect()
}

async fn import<P, C>(cached: &Cached<P, C>, content: &[u8])
where
    Cached<P, C>: Provider<dialog_effects::blob::Import>,
{
    let mut sink = Provider::<dialog_effects::blob::Import>::execute(
        cached,
        subject()
            .writer()
            .archive()
            .blob()
            .import(Blake3Hash::hash(content), content.len() as u64),
    )
    .await
    .unwrap();
    sink.write_all(content).await.unwrap();
    sink.finish().await.unwrap();
}

#[tokio::test]
async fn an_imported_blob_is_read_whole_and_in_ranges_from_the_cache() {
    let cache = MemoryCache::default();
    let content = blob();
    let digest = Blake3Hash::hash(&content);

    let writer = Cached::new(MemoryStore::default(), cache.clone());
    import(&writer, &content).await;
    assert_eq!(writer.outcome(), Outcome::Fill);
    settle(&writer).await;

    let reader = Cached::new(MemoryStore::default(), cache.clone());
    let whole = Provider::<dialog_effects::blob::Read>::execute(
        &reader,
        subject().reader().archive().blob().read(digest.clone()),
    )
    .await
    .unwrap();
    assert_eq!(collect(whole).await.unwrap(), content);
    assert_eq!(reader.outcome(), Outcome::Hit);

    let ranged = Provider::<dialog_effects::blob::Read>::execute(
        &reader,
        subject()
            .reader()
            .archive()
            .blob()
            .invoke(Read::range(digest, 1_000, Some(500))),
    )
    .await
    .unwrap();
    assert_eq!(collect(ranged).await.unwrap(), &content[1_000..1_500]);
    assert_eq!(reader.outcome(), Outcome::Hit);
}

#[tokio::test]
async fn a_ranged_miss_fills_nothing_and_a_whole_miss_fills_the_blob() {
    let cache = MemoryCache::default();
    let store = MemoryStore::default();
    let content = blob();
    let digest = Blake3Hash::hash(&content);
    import(
        &Cached::new(store.clone(), MemoryCache::default()),
        &content,
    )
    .await;
    let key = blob_key(
        &subject().reader().archive().blob().read(digest.clone()),
        &digest,
    );

    let cached = Cached::new(store, cache.clone());
    let ranged = Provider::<dialog_effects::blob::Read>::execute(
        &cached,
        subject()
            .reader()
            .archive()
            .blob()
            .invoke(Read::range(digest.clone(), 0, Some(10))),
    )
    .await
    .unwrap();
    assert_eq!(collect(ranged).await.unwrap(), &content[..10]);
    assert_eq!(cached.outcome(), Outcome::Miss);
    settle(&cached).await;
    assert!(!cache.holds(&key), "a range never fills the cache");

    let whole = Provider::<dialog_effects::blob::Read>::execute(
        &cached,
        subject().reader().archive().blob().read(digest),
    )
    .await
    .unwrap();
    assert_eq!(collect(whole).await.unwrap(), content);
    assert_eq!(cached.outcome(), Outcome::Miss);
    settle(&cached).await;
    assert!(
        cache.holds(&key),
        "a whole read fills the cache behind the answer"
    );
}

#[tokio::test]
async fn cells_never_touch_the_cache() {
    let cache = MemoryCache::default();
    let cached = Cached::new(MemoryStore::default(), cache.clone());
    let writable = || subject().writer().memory().space("sync").cell("head");
    let readable = || subject().reader().memory().space("sync").cell("head");
    let version = Provider::<dialog_effects::memory::Publish>::execute(
        &cached,
        writable().publish(b"one".to_vec(), None),
    )
    .await
    .unwrap();
    assert_eq!(cached.outcome(), Outcome::None);
    let resolved =
        Provider::<dialog_effects::memory::Resolve>::execute(&cached, readable().resolve())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(resolved.version, version);
    assert_eq!(cached.outcome(), Outcome::None);
    assert!(cached.take_fills().is_empty());
    assert!(cache.0.lock().unwrap().is_empty());
}
