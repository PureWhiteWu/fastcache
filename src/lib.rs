//! A performant but not-so-accurate time and capacity based cache for Rust.
//!
//! This module provides an implementation of a time-to-live (TTL) and capacity based cache.
//! It stores key-value pairs and automatically evicts expired entries based on their TTL.
//!
//! The implementation trades off exact TTL-based expiration for better performance,
//! which means that expired items may not be removed immediately upon expiration, and an
//! item may be removed before its expiration time.
//!
//! The implmentation also may return expired items, which means that the caller should
//! check the expiration status of the returned value by calling `value.is_expired()`
//! before using it. It's up to the user if to use the expired value or not. This design
//! can be useful in some cases, for example, when the caller wants to use the expired
//! value as a fallback or to do rpc in background to update the value and return the
//! expired value immediately to reduce latency.
//!
//! # Examples
//!
//! ```
//! use ttl_cache::Cache;
//! use std::time::Duration;
//!
//! let cache = Cache::new(3, Duration::from_secs(3600)); // Capacity: 3, TTL: 1 hour
//! cache.insert("key1", "value1");
//! cache.insert("key2", "value2");
//!
//! if let Some(value) = cache.get("key1") {
//!     println!("Value: {}", value.get());
//! } else {
//!     println!("Value not found");
//! }
//! ```

use std::{
    hash::Hash,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use crossbeam_queue::ArrayQueue;
use crossbeam_utils::{atomic::AtomicCell, CachePadded};
use dashmap::DashMap;

/// Represents an entry in the cache.
///
/// Wraps a value with an expiration timestamp and an expired flag.
pub struct Value<V> {
    value: V,
    expire_at: Instant,
    is_expired: bool,
}

impl<V> Value<V> {
    /// Get a reference to the inner value.
    pub fn get(&self) -> &V {
        &self.value
    }

    /// Get a mutable reference to the inner value.
    pub fn get_mut(&mut self) -> &mut V {
        &mut self.value
    }

    /// Consumes the `Value` and returns its inner value.
    pub fn into_inner(self) -> V {
        self.value
    }

    /// Check if the value is expired.
    pub fn is_expired(&self) -> bool {
        self.is_expired
    }

    /// Get the expiration timestamp of the value.
    pub fn expire_at(&self) -> Instant {
        self.expire_at
    }
}

impl<V> std::ops::Deref for Value<V> {
    type Target = V;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<V> std::ops::DerefMut for Value<V> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

/// A not so accurate but performant time and capacity based cache.
pub struct Cache<K, V> {
    map: DashMap<K, (V, Instant), ahash::RandomState>,
    ringbuf: ArrayQueue<(K, Instant)>,

    capacity: usize,
    ttl: Duration,

    expire_started: CachePadded<AtomicBool>,
    oldest: CachePadded<AtomicCell<Instant>>,
}

impl<K, V> Cache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create a new cache with the given capacity and time-to-live (TTL) for values.
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            map: DashMap::with_capacity_and_hasher(capacity, ahash::RandomState::new()),
            ringbuf: ArrayQueue::new(capacity),
            capacity,
            ttl,
            expire_started: CachePadded::new(AtomicBool::new(false)),
            oldest: CachePadded::new(AtomicCell::new(Instant::now())),
        }
    }

    /// Get the number of elements in the cache.
    pub fn len(&self) -> usize {
        self.ringbuf.len()
    }

    /// Get the capacity of the cache.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Get the value associated with the given key, if it exists and is not expired.
    pub fn get(&self, key: K) -> Option<Value<V>> {
        let v = self.map.get(&key);
        if v.is_none() {
            return None;
        }
        let v = v.unwrap();
        let now = Instant::now();
        let value = Value {
            value: v.0.clone(),
            expire_at: v.1,
            is_expired: now > v.1,
        };
        self.do_expire(now);
        Some(value)
    }

    /// Insert a key-value pair in the cache.
    ///
    /// If the cache is full, it will evict the oldest entry.
    pub fn insert(&self, key: K, value: V) {
        let now = Instant::now();
        let expire_at = now + self.ttl;
        while let Err(_) = self.ringbuf.push((key.clone(), expire_at)) {
            // ringbuf is full, pop one
            let (k, e) = self.ringbuf.pop().unwrap();
            self.map.remove(&k);
            self.oldest.store(e);
        }
        self.map.insert(key, (value, expire_at));
        self.do_expire(now);
    }

    /// Check and evict expired items in the cache.
    fn do_expire(&self, now: Instant) {
        if self.oldest.load() > now {
            // don't need to do expire
            return;
        }

        // grab the lock, a simple singleflight implementation
        if self
            .expire_started
            .compare_exchange_weak(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            return;
        }

        while let Some((k, t)) = self.ringbuf.pop() {
            self.map.remove(&k);
            if now <= t {
                // TODO: find a way to put it back, or peek it instead of pop.
                self.oldest.store(t);
                break;
            }
        }
        self.expire_started.store(false, Ordering::Release);
    }
}
