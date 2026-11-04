//! The cache of blocks of a guest kernel: the blocks that it read or wrote, by
//! layer, catalog, and hash, so that a read of the same block again makes no
//! call of the world. See PDD007, "The Cost of a Read".
//!
//! A block is immutable: its key is its hash, and the daemon takes no block
//! under another hash. So the cache never goes stale. It keeps only a block
//! that the guest kernel read from the layer or wrote to it: a get that finds
//! none is not kept. A block that the layer drops later, or that the handle can
//! no longer read once its chain expires, can stay until it goes out of the
//! cache; the worker had it already, so the cache shows it nothing new. Each
//! layer has its own part of the cache, so that a read of one layer never finds
//! a block of another, which its scope may not allow.
//!
//! The cache is bounded by the bytes that it keeps, [`MOST_BYTES`], so that
//! it stays well under the memory ceiling of a worker. Past the bound, the
//! block that was read longest ago goes first. A block longer than
//! [`LONGEST`] is not kept.

use std::collections::{BTreeMap, HashMap};

/// The most bytes of blocks that the cache keeps: 32 MiB, under the 768 MiB
/// of linear memory that a guest kernel of the default memory ceiling has.
pub const MOST_BYTES: usize = 32 * 1024 * 1024;

/// The longest block that the cache keeps: 1 MiB. A node of a tree of Dialog
/// DB is a few KiB.
pub const LONGEST: usize = 1024 * 1024;

/// The bytes that the cache counts for each block besides the bytes of the
/// block: its key, and the room of its entries.
const OVERHEAD: usize = 128;

/// A block by its layer, its catalog, and its hash.
type Key = (u64, String, Vec<u8>);

/// The cache of blocks. See the module.
pub struct Blocks {
    entries: HashMap<Key, Entry>,
    /// Each key, by the tick of its last read: the first goes first.
    order: BTreeMap<u64, Key>,
    tick: u64,
    bytes: usize,
    most: usize,
}

struct Entry {
    block: Vec<u8>,
    tick: u64,
}

impl Blocks {
    /// An empty cache of at most `most` bytes.
    pub fn new(most: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            bytes: 0,
            most,
        }
    }

    /// The block of `key` in `catalog` of the layer `layer`, if the cache
    /// keeps it.
    pub fn get(&mut self, layer: u64, catalog: &str, key: &[u8]) -> Option<Vec<u8>> {
        let id = (layer, catalog.to_owned(), key.to_vec());
        let tick = self.next();
        let entry = self.entries.get_mut(&id)?;
        let key = self
            .order
            .remove(&entry.tick)
            .expect("each entry has its tick");
        entry.tick = tick;
        self.order.insert(tick, key);
        Some(entry.block.clone())
    }

    /// Keeps `block`, of `key` in `catalog` of the layer `layer`, unless it
    /// is longer than [`LONGEST`] or than the whole cache.
    pub fn put(&mut self, layer: u64, catalog: &str, key: &[u8], block: &[u8]) {
        let cost = block.len() + key.len() + catalog.len() + OVERHEAD;
        if block.len() > LONGEST || cost > self.most {
            return;
        }
        let id = (layer, catalog.to_owned(), key.to_vec());
        if self.entries.contains_key(&id) {
            return;
        }
        while self.bytes + cost > self.most {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            self.remove(&oldest);
        }
        let tick = self.next();
        self.order.insert(tick, id.clone());
        self.entries.insert(
            id,
            Entry {
                block: block.to_vec(),
                tick,
            },
        );
        self.bytes += cost;
    }

    /// Forgets each block of the layer `layer`, which the guest kernel dropped.
    pub fn forget(&mut self, layer: u64) {
        let gone: Vec<Key> = self
            .entries
            .keys()
            .filter(|(of, _, _)| *of == layer)
            .cloned()
            .collect();
        for id in gone {
            if let Some(entry) = self.entries.get(&id) {
                self.order.remove(&entry.tick);
            }
            self.remove(&id);
        }
    }

    /// Removes the entry of `id`, whose tick is out of the order already.
    fn remove(&mut self, id: &Key) {
        if let Some(entry) = self.entries.remove(id) {
            self.bytes -= entry.block.len() + id.1.len() + id.2.len() + OVERHEAD;
        }
    }

    fn next(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_gives_a_block_back_by_its_layer_catalog_and_key() {
        let mut blocks = Blocks::new(MOST_BYTES);
        blocks.put(1, "index", b"key", b"block");

        assert_eq!(blocks.get(1, "index", b"key"), Some(b"block".to_vec()));
        assert_eq!(blocks.get(2, "index", b"key"), None, "another layer");
        assert_eq!(blocks.get(1, "other", b"key"), None, "another catalog");
        assert_eq!(blocks.get(1, "index", b"yek"), None, "another key");
    }

    #[test]
    fn it_drops_the_block_read_longest_ago_past_its_bound() {
        let block = vec![7; 1000];
        let cost = block.len() + 3 + "index".len() + OVERHEAD;
        let mut blocks = Blocks::new(3 * cost);
        blocks.put(1, "index", b"one", &block);
        blocks.put(1, "index", b"two", &block);
        blocks.put(1, "index", b"thr", &block);
        // A read of the first makes the second the one read longest ago.
        assert!(blocks.get(1, "index", b"one").is_some());

        blocks.put(1, "index", b"fou", &block);

        assert_eq!(blocks.len(), 3);
        assert!(blocks.bytes() <= 3 * cost);
        assert!(blocks.get(1, "index", b"two").is_none());
        for kept in [b"one", b"thr", b"fou"] {
            assert!(blocks.get(1, "index", kept).is_some());
        }
    }

    #[test]
    fn it_keeps_no_block_longer_than_its_longest() {
        let mut blocks = Blocks::new(MOST_BYTES);
        blocks.put(1, "index", b"key", &vec![0; LONGEST + 1]);

        assert!(blocks.is_empty());
        assert_eq!(blocks.bytes(), 0);
    }

    #[test]
    fn it_forgets_each_block_of_a_dropped_layer() {
        let mut blocks = Blocks::new(MOST_BYTES);
        blocks.put(1, "index", b"one", b"block");
        blocks.put(2, "index", b"two", b"block");

        blocks.forget(1);

        assert!(blocks.get(1, "index", b"one").is_none());
        assert!(blocks.get(2, "index", b"two").is_some());
        assert_eq!(blocks.len(), 1);
    }
}
