use std::{
    hash::Hash,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use crossbeam_queue::ArrayQueue;
use crossbeam_utils::{atomic::AtomicCell, CachePadded};
use dashmap::DashMap;

pub struct Value<V> {
    value: V,
    expire_at: Instant,
    is_expired: bool,
}

impl<V> Value<V> {
    pub fn get(&self) -> &V {
        &self.value
    }

    pub fn get_mut(&mut self) -> &mut V {
        &mut self.value
    }

    pub fn into_inner(self) -> V {
        self.value
    }

    pub fn is_expired(&self) -> bool {
        self.is_expired
    }

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

    pub fn len(&self) -> usize {
        self.ringbuf.len()
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

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
