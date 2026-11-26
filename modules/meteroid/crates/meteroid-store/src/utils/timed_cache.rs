use cached::{Cached, TimedSizedCache};
use std::hash::Hash;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

/// A size-bounded cache whose entries age out, held by its store and shared by its clones.
/// Never process-wide: stores sharing a process (one per test, each on its own database, seeded
/// with the same ids) each read their own database.
pub struct TimedCache<K, V> {
    entries: Arc<Mutex<TimedSizedCache<K, V>>>,
}

impl<K, V> Clone for TimedCache<K, V> {
    fn clone(&self) -> Self {
        Self {
            entries: self.entries.clone(),
        }
    }
}

impl<K: Hash + Eq + Clone, V: Clone> TimedCache<K, V> {
    pub fn new(size: usize, lifespan: Duration) -> Self {
        Self {
            entries: Arc::new(Mutex::new(TimedSizedCache::with_size_and_lifespan(
                size, lifespan,
            ))),
        }
    }

    fn entries(&self) -> MutexGuard<'_, TimedSizedCache<K, V>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn get(&self, key: &K) -> Option<V> {
        self.entries().cache_get(key).cloned()
    }

    pub fn insert(&self, key: K, value: V) {
        self.entries().cache_set(key, value);
    }

    pub fn remove(&self, key: &K) {
        self.entries().cache_remove(key);
    }

    pub fn clear(&self) {
        self.entries().cache_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::TimedCache;
    use std::time::Duration;

    #[test]
    fn caches_of_two_stores_are_apart_and_clones_share_one() {
        let a: TimedCache<u8, &str> = TimedCache::new(10, Duration::from_secs(60));
        let b: TimedCache<u8, &str> = TimedCache::new(10, Duration::from_secs(60));
        a.insert(1, "a");
        assert_eq!(a.clone().get(&1), Some("a"));
        assert_eq!(b.get(&1), None);
        a.clone().remove(&1);
        assert_eq!(a.get(&1), None);
    }
}
