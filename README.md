# fastcache

A performant but not-so-accurate time and capacity based cache for Rust.

This module provides an implementation of a time-to-live (TTL) and capacity based cache.
It stores key-value pairs and automatically evicts expired entries based on their TTL.

The implementation trades off exact TTL-based expiration for better performance,
which means that expired items may not be removed immediately upon expiration, and an
item may be removed before its expiration time.

The implmentation also may return expired items, which means that the caller should
check the expiration status of the returned value by calling `value.is_expired()`
before using it. It's up to the user if to use the expired value or not. This design
can be useful in some cases, for example, when the caller wants to use the expired
value as a fallback or to do rpc in background to update the value and return the
expired value immediately to reduce latency.

# Examples

```rust
use fastcache::Cache;
use std::time::Duration;

let cache = Cache::new(3, Duration::from_secs(3600)); // Capacity: 3, TTL: 1 hour
cache.insert("key1", "value1");
cache.insert("key2", "value2");

if let Some(value) = cache.get("key1") {
    println!("Value: {}", value.get());
} else {
    println!("Value not found");
}
```
