//! LRU mesh cache — holds MESH1 bytes keyed by `(BodyId, Lod, generation)`.
//!
//! The `generation` pins a mesh to the snapshot that produced it (plan "Mesh
//! transfer": LRU `MeshCache` keyed `(BodyId, Lod, generation)`), so a stale
//! mesh is never returned for a newer snapshot. Bytes are held behind an `Arc`
//! so a `get_mesh` command hands the webview a zero-copy `tauri::ipc::Response`
//! without re-cloning the blob.
//!
//! The bytes are **opaque** to Rust (MESH1 travels verbatim end-to-end,
//! Invariant 5) — the cache never parses or re-encodes them.

use std::collections::HashMap;
use std::sync::Arc;

use onecad_core::regen::MeshKey;

/// Default entry capacity. A regen seeds the cache with EVERY body it published
/// (the worker inlines their MESH1 blobs into the `ExecutePlan` terminal), so the
/// bound has to cover a whole document's body count, not just the handful a pull
/// model would touch. The byte budget below is what actually caps memory.
pub const DEFAULT_CAPACITY: usize = 512;

/// Default byte budget (256 MiB). The entry cap alone cannot bound memory once a
/// whole publish is seeded at once — a few hundred dense bodies would dwarf it.
pub const DEFAULT_BYTE_CAPACITY: usize = 256 * 1024 * 1024;

/// A bounded LRU cache of MESH1 blobs.
///
/// Eviction is strict LRU: `get` and `put` mark a key most-recently-used; the
/// least-recently-used key is dropped until BOTH bounds hold — at most `capacity`
/// entries AND at most `byte_capacity` bytes of blob.
///
/// The most-recently-used entry is never evicted for the byte bound, so a single
/// blob larger than the whole budget is still cached (and evicted by the next
/// `put`). Refusing it would make `put` silently a no-op for exactly the meshes a
/// re-fetch is most expensive for.
#[derive(Debug)]
pub struct MeshCache {
    capacity: usize,
    byte_capacity: usize,
    /// Sum of `map`'s blob lengths — maintained by `put`/eviction/`clear`.
    bytes: usize,
    map: HashMap<MeshKey, Arc<Vec<u8>>>,
    /// Keys in ascending recency (front = LRU, back = MRU).
    order: Vec<MeshKey>,
}

impl MeshCache {
    /// A cache with [`DEFAULT_CAPACITY`] / [`DEFAULT_BYTE_CAPACITY`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CAPACITY)
    }

    /// A cache holding at most `capacity` entries (minimum 1), with the default
    /// byte budget.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::with_limits(capacity, DEFAULT_BYTE_CAPACITY)
    }

    /// A cache bounded by both an entry count (minimum 1) and a byte budget.
    #[must_use]
    pub fn with_limits(capacity: usize, byte_capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            byte_capacity,
            bytes: 0,
            map: HashMap::new(),
            order: Vec::new(),
        }
    }

    /// Total cached blob bytes.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.bytes
    }

    /// Number of cached entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// True iff nothing is cached.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Whether `key` is currently cached (does NOT bump recency).
    #[must_use]
    pub fn contains(&self, key: &MeshKey) -> bool {
        self.map.contains_key(key)
    }

    /// Fetches a blob, marking it most-recently-used. `None` on a miss.
    pub fn get(&mut self, key: &MeshKey) -> Option<Arc<Vec<u8>>> {
        let hit = self.map.get(key).cloned();
        if hit.is_some() {
            self.touch(key);
        }
        hit
    }

    /// Fetches a blob **without** touching recency — for an observer (a test, a
    /// diagnostic) that must not perturb the eviction order it is inspecting.
    #[must_use]
    pub fn peek(&self, key: &MeshKey) -> Option<Arc<Vec<u8>>> {
        self.map.get(key).cloned()
    }

    /// Inserts (or replaces) a blob, marking it most-recently-used and evicting
    /// least-recently-used entries until both bounds hold.
    pub fn put(&mut self, key: MeshKey, bytes: Arc<Vec<u8>>) {
        let added = bytes.len();
        if let Some(replaced) = self.map.insert(key, bytes) {
            // Replacing an existing key: drop the old blob's bytes from the total,
            // else the budget drifts up by every overwrite (and evicts live entries).
            self.bytes -= replaced.len();
            self.touch(&key);
        } else {
            self.order.push(key);
        }
        self.bytes += added;
        self.evict();
    }

    /// Drops every entry (document close).
    pub fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
        self.bytes = 0;
    }

    /// Drops LRU entries until the entry count AND the byte budget both hold. The
    /// MRU entry is never dropped (see the type docs).
    fn evict(&mut self) {
        while self.map.len() > self.capacity
            || (self.bytes > self.byte_capacity && self.map.len() > 1)
        {
            if self.order.is_empty() {
                break;
            }
            let evicted = self.order.remove(0);
            if let Some(blob) = self.map.remove(&evicted) {
                self.bytes -= blob.len();
            }
        }
    }

    /// Moves `key` to the MRU end of the recency order.
    fn touch(&mut self, key: &MeshKey) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let k = self.order.remove(pos);
            self.order.push(k);
        }
    }
}

impl Default for MeshCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use onecad_core::ids::BodyId;
    use onecad_core::regen::Lod;
    use uuid::Uuid;

    fn key(n: u128, gen: u64) -> MeshKey {
        MeshKey {
            body: BodyId(Uuid::from_u128(n)),
            lod: Lod::Coarse,
            generation: gen,
        }
    }

    #[test]
    fn miss_then_hit() {
        let mut c = MeshCache::new();
        let k = key(1, 1);
        assert!(c.get(&k).is_none(), "cold miss");
        let bytes = Arc::new(vec![1u8, 2, 3]);
        c.put(k, bytes.clone());
        let got = c.get(&k).expect("hit");
        assert_eq!(*got, vec![1, 2, 3]);
        assert!(
            Arc::ptr_eq(&got, &bytes),
            "same Arc handed back (zero-copy)"
        );
    }

    #[test]
    fn distinct_generation_is_a_distinct_entry() {
        let mut c = MeshCache::new();
        c.put(key(1, 1), Arc::new(vec![1]));
        c.put(key(1, 2), Arc::new(vec![2]));
        assert_eq!(c.len(), 2, "generation is part of the key");
        assert_eq!(*c.get(&key(1, 1)).unwrap(), vec![1]);
        assert_eq!(*c.get(&key(1, 2)).unwrap(), vec![2]);
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut c = MeshCache::with_capacity(2);
        c.put(key(1, 1), Arc::new(vec![1]));
        c.put(key(2, 1), Arc::new(vec![2]));
        // Touch key 1 so key 2 becomes the LRU victim.
        let _ = c.get(&key(1, 1));
        c.put(key(3, 1), Arc::new(vec![3]));
        assert_eq!(c.len(), 2);
        assert!(c.contains(&key(1, 1)), "recently used survives");
        assert!(!c.contains(&key(2, 1)), "LRU evicted");
        assert!(c.contains(&key(3, 1)));
    }

    #[test]
    fn clear_drops_everything() {
        let mut c = MeshCache::new();
        c.put(key(1, 1), Arc::new(vec![1]));
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.byte_len(), 0, "clear resets the byte total too");
    }

    #[test]
    fn byte_budget_evicts_before_entry_cap() {
        // Room for 100 entries but only 250 bytes: the byte bound must bite first.
        let mut c = MeshCache::with_limits(100, 250);
        c.put(key(1, 1), Arc::new(vec![0u8; 100]));
        c.put(key(2, 1), Arc::new(vec![0u8; 100]));
        assert_eq!(c.len(), 2, "under both bounds");
        c.put(key(3, 1), Arc::new(vec![0u8; 100]));
        assert_eq!(c.len(), 2, "byte budget evicted while entry count had room");
        assert!(!c.contains(&key(1, 1)), "LRU dropped for the byte budget");
        assert!(c.contains(&key(2, 1)));
        assert!(c.contains(&key(3, 1)));
        assert_eq!(c.byte_len(), 200);
    }

    #[test]
    fn oversized_single_blob_is_still_cached() {
        let mut c = MeshCache::with_limits(100, 10);
        c.put(key(1, 1), Arc::new(vec![0u8; 4096]));
        assert!(
            c.contains(&key(1, 1)),
            "the MRU entry is never evicted for the byte bound"
        );
    }

    #[test]
    fn replacing_a_key_adjusts_the_byte_total() {
        let mut c = MeshCache::with_limits(100, 1000);
        c.put(key(1, 1), Arc::new(vec![0u8; 400]));
        assert_eq!(c.byte_len(), 400);
        c.put(key(1, 1), Arc::new(vec![0u8; 100]));
        assert_eq!(c.len(), 1, "same key, still one entry");
        assert_eq!(c.byte_len(), 100, "the replaced blob's bytes are released");
        // The released headroom is real: two more 400-byte blobs still fit.
        c.put(key(2, 1), Arc::new(vec![0u8; 400]));
        c.put(key(3, 1), Arc::new(vec![0u8; 400]));
        assert_eq!(c.len(), 3, "no spurious eviction from a drifted total");
    }

    #[test]
    fn peek_does_not_touch_recency() {
        let mut c = MeshCache::with_capacity(2);
        c.put(key(1, 1), Arc::new(vec![1]));
        c.put(key(2, 1), Arc::new(vec![2]));
        // `get` would promote key 1; `peek` must not.
        assert_eq!(*c.peek(&key(1, 1)).expect("peek hit"), vec![1]);
        c.put(key(3, 1), Arc::new(vec![3]));
        assert!(
            !c.contains(&key(1, 1)),
            "key 1 stayed LRU across the peek and was evicted"
        );
        assert!(c.contains(&key(2, 1)));
        assert!(c.contains(&key(3, 1)));
        assert!(c.peek(&key(1, 1)).is_none(), "peek misses cleanly");
    }
}
