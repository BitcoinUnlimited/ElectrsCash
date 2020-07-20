use indexmap::IndexMap;
use prometheus::{IntCounterVec, IntGauge};
use rand::prelude::*;
use std::hash::Hash;

pub struct RndCache<K: Eq + Hash, V> {
    map: IndexMap<K, (u32, V)>,
    bytes_capacity: u64,
    bytes_used: u64,
    rng: StdRng,
    entry_overhead: u32,

    /// How many hits or misses
    metric_lookups: IntCounterVec,
    /// How many inserts and evictions
    metric_churn: IntCounterVec,
    /// How much cache is in use (in bytes)
    metric_size: IntGauge,
    /// How many elements are cached
    metric_entries: IntGauge,
}

impl<K: Eq + Hash, V> RndCache<K, V> {
    pub fn new(
        bytes_capacity: u64,
        metric_lookups: IntCounterVec,
        metric_churn: IntCounterVec,
        metric_size: IntGauge,
        metric_entries: IntGauge,
    ) -> RndCache<K, V> {
        // We need an guessestimate container overhead there is for each
        // element.
        //
        // We know that IndexMap stores an internal usize hash value. We also
        // store the size of the entry as u32.
        //
        // There is also some unknown
        let entry_overhead = std::mem::size_of::<usize>() + std::mem::size_of::<u32>()
            + /* unknown extra */ std::mem::size_of::<u32>();

        RndCache {
            map: IndexMap::new(),
            bytes_capacity,
            bytes_used: 0,
            rng: StdRng::seed_from_u64(42),
            entry_overhead: entry_overhead as u32,
            metric_lookups,
            metric_size,
            metric_entries,
            metric_churn,
        }
    }

    fn dec_bytes_used(&mut self, entry_size: u32) {
        self.bytes_used -= (entry_size + self.entry_overhead) as u64;
        self.metric_size.set(self.bytes_used as i64);
    }

    fn inc_bytes_used(&mut self, entry_size: u32) {
        self.bytes_used += (entry_size + self.entry_overhead) as u64;
        self.metric_size.set(self.bytes_used as i64);
    }

    pub fn override_entry_overhead(&mut self, size: u32) {
        debug_assert!(self.map.is_empty());
        self.entry_overhead = size;
    }

    pub fn put(&mut self, k: K, v: V, size: u64) {
        if size > self.bytes_capacity {
            return;
        }

        if size + self.entry_overhead as u64 > std::u32::MAX as u64 {
            // Cache does not support entries of this size.
            return;
        }
        let size = size as u32;

        while !self.fits_in_cache(size) {
            self.evict_random();
        }

        match self.map.insert(k, (size, v)) {
            Some(v) => {
                // key existed and value was replaced
                let (old_size, _) = v;
                self.dec_bytes_used(old_size);
            }
            None => {
                self.metric_churn.with_label_values(&["inserted"]).inc();
            }
        };
        self.inc_bytes_used(size);
        self.metric_entries.set(self.map.len() as i64);
    }

    pub fn get(&self, k: &K) -> Option<&V> {
        match self.map.get(k) {
            Some(v) => {
                self.metric_lookups.with_label_values(&["hit"]).inc();
                let (_, value) = v;
                Some(value)
            }
            None => {
                self.metric_lookups.with_label_values(&["miss"]).inc();
                None
            }
        }
    }

    pub fn usage(&self) -> u64 {
        self.bytes_used
    }

    pub fn capacity(&self) -> u64 {
        self.bytes_capacity
    }

    fn fits_in_cache(&self, bytes: u32) -> bool {
        self.bytes_used + bytes as u64 <= self.bytes_capacity
    }

    /// Removes a random cache entry
    fn evict_random(&mut self) {
        let index = self.rng.gen_range(0, self.map.len());
        let (_, (size, _)) = self.map.swap_remove_index(index).unwrap();
        self.dec_bytes_used(size);
        self.metric_churn.with_label_values(&["evicted"]).inc();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_int_vec_counter() -> IntCounterVec {
        IntCounterVec::new(prometheus::Opts::new("name", "help"), &["type"]).unwrap()
    }

    fn dummy_int_gauge() -> IntGauge {
        IntGauge::new("usage", "help").unwrap()
    }

    #[test]
    fn test_insert_newitem() {
        let mut cache: RndCache<i32, i32> = RndCache::new(
            100,
            dummy_int_vec_counter(),
            dummy_int_vec_counter(),
            dummy_int_gauge(),
            dummy_int_gauge(),
        );
        cache.override_entry_overhead(0);
        cache.put(10, 10, 10);
        assert_eq!(&10, cache.get(&10).unwrap());
        assert!(!cache.get(&20).is_some());
        cache.put(20, 20, 20);
        assert_eq!(&10, cache.get(&10).unwrap());
        assert_eq!(&20, cache.get(&20).unwrap());

        assert_eq!(30, cache.usage());
    }

    #[test]
    fn test_insert_replace() {
        let mut cache: RndCache<i32, i32> = RndCache::new(
            100,
            dummy_int_vec_counter(),
            dummy_int_vec_counter(),
            dummy_int_gauge(),
            dummy_int_gauge(),
        );
        cache.override_entry_overhead(0);
        cache.put(10, 10, 10);
        assert_eq!(&10, cache.get(&10).unwrap());
        assert_eq!(10, cache.usage());

        cache.put(10, 20, 20);
        assert_eq!(&20, cache.get(&10).unwrap());
        assert_eq!(20, cache.usage());
    }

    #[test]
    fn test_too_big() {
        let capacity = 100;
        let mut cache: RndCache<i32, i32> = RndCache::new(
            capacity,
            dummy_int_vec_counter(),
            dummy_int_vec_counter(),
            dummy_int_gauge(),
            dummy_int_gauge(),
        );

        cache.override_entry_overhead(0);
        cache.put(10, 10, capacity + 1);
        assert!(!cache.get(&10).is_some());

        cache.put(10, 10, capacity);
        assert!(cache.get(&10).is_some());

        cache.put(10, 10, capacity - 1);
        assert!(cache.get(&10).is_some());
    }

    #[test]
    fn test_capacity() {
        let mut cache: RndCache<&str, i32> = RndCache::new(
            300,
            dummy_int_vec_counter(),
            dummy_int_vec_counter(),
            dummy_int_gauge(),
            dummy_int_gauge(),
        );
        cache.override_entry_overhead(0);
        assert_eq!(300, cache.capacity());
        assert_eq!(0, cache.usage());
        cache.put("key1", 10, 100);
        assert_eq!(100, cache.usage());

        // replace cache entry
        cache.put("key1", 10, 150);
        assert_eq!(150, cache.usage());

        // new entry
        cache.put("key2", 10, 60);
        assert_eq!(210, cache.usage());

        // to make space for next entry, both previous entries need
        // to be evicted
        cache.put("key3", 10, 250);
        assert_eq!(250, cache.usage());
    }

    fn count_hits(cache: &RndCache<&str, i32>, keys: Vec<&str>) -> u64 {
        let mut hits = 0;
        for k in keys {
            if cache.get(&k).is_some() {
                hits += 1;
            }
        }
        hits
    }

    #[test]
    fn test_evict() {
        let capacity = 300;

        let mut cache: RndCache<&str, i32> = RndCache::new(
            capacity,
            dummy_int_vec_counter(),
            dummy_int_vec_counter(),
            dummy_int_gauge(),
            dummy_int_gauge(),
        );

        cache.override_entry_overhead(0);

        // fill cache
        cache.put("key1", 1, 100);
        cache.put("key2", 2, 100);
        cache.put("key3", 3, 100);
        assert_eq!(cache.capacity(), cache.usage());
        assert_eq!(3, count_hits(&cache, vec!("key1", "key2", "key3")));

        // evict 1
        cache.put("key4", 4, 100);
        assert_eq!(2, count_hits(&cache, vec!("key1", "key2", "key3")));

        // evict all
        cache.put("key5", 5, capacity);
        assert_eq!(0, count_hits(&cache, vec!("key1", "key2", "key3")));
    }
}
