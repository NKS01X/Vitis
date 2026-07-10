// Exclusive tests for the `vitis-cache` crate: content-addressed cache + diff.

use std::collections::HashMap;

use vitis_cache::{CacheKey, ContentCache};

#[test]
fn key_is_deterministic() {
    let a = CacheKey::new("RUN make", &["x".into()]);
    let b = CacheKey::new("RUN make", &["x".into()]);
    assert_eq!(a, b);
    assert_eq!(a.hash, b.hash);
}

#[test]
fn key_hash_is_64_hex_chars() {
    let k = CacheKey::new("x", &[]);
    assert_eq!(k.hash.len(), 64);
    assert!(k.hash.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn key_changes_with_instruction() {
    let a = CacheKey::new("RUN make", &["x".into()]);
    let b = CacheKey::new("RUN build", &["x".into()]);
    assert_ne!(a.hash, b.hash);
}

#[test]
fn key_changes_with_parents() {
    let a = CacheKey::new("RUN make", &["x".into()]);
    let b = CacheKey::new("RUN make", &["y".into()]);
    assert_ne!(a.hash, b.hash);
}

#[test]
fn key_records_parent_hashes() {
    let k = CacheKey::new("RUN", &["p1".into(), "p2".into()]);
    assert_eq!(k.parent_hashes, vec!["p1".to_string(), "p2".to_string()]);
}

#[test]
fn empty_instruction_still_hashes() {
    let k = CacheKey::new("", &[]);
    assert!(!k.hash.is_empty());
}

#[test]
fn store_and_load_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    let key = CacheKey::new("RUN echo hi", &[]);
    cache.store(&key, b"artifact").unwrap();
    assert!(cache.hit(&key));
    assert_eq!(cache.load(&key).unwrap(), b"artifact");
}

#[test]
fn miss_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    let key = CacheKey::new("nope", &[]);
    assert!(!cache.hit(&key));
    assert!(cache.load(&key).is_none());
}

#[test]
fn store_creates_nested_dir() {
    let dir = tempfile::tempdir().unwrap().into_path().join("nested/deep");
    let cache = ContentCache::new(&dir);
    cache.store(&CacheKey::new("k", &[]), b"x").unwrap();
    assert!(dir.exists());
}

#[test]
fn diff_finds_single_range() {
    let d = ContentCache::diff(b"hello world", b"hello vitis");
    assert_eq!(d, vec![(6, 11)]);
}

#[test]
fn diff_identical_is_empty() {
    assert!(ContentCache::diff(b"same", b"same").is_empty());
}

#[test]
fn diff_prefix_only() {
    let d = ContentCache::diff(b"abcdef", b"abXXXX");
    assert_eq!(d, vec![(2, 6)]);
}

#[test]
fn diff_trailing_only() {
    let d = ContentCache::diff(b"fixed", b"fixedextra");
    assert_eq!(d, vec![(5, 10)]);
}

#[test]
fn diff_different_lengths() {
    let d = ContentCache::diff(b"abc", b"abcd");
    assert_eq!(d, vec![(3, 4)]);
}

#[test]
fn diff_common_substring_skipped() {
    let d = ContentCache::diff(b"AAAA BBBB CCCC", b"XXXX BBBB YYYY");
    // "BBBB" at offset 5 matches in both -> only head and tail differ
    assert_eq!(d, vec![(0, 4), (10, 14)]);
}

#[test]
fn diff_size_counts_bytes() {
    assert_eq!(ContentCache::diff_size(b"aaaa", b"bbbb"), 4);
    assert_eq!(ContentCache::diff_size(b"same", b"same"), 0);
}

#[test]
fn diff_size_matches_ranges() {
    let a = b"the quick brown fox".to_vec();
    let b = b"the slow brown fox".to_vec();
    let size = ContentCache::diff_size(&a, &b);
    let ranges: usize = ContentCache::diff(&a, &b).iter().map(|(s, e)| e - s).sum();
    assert_eq!(size, ranges);
}

#[test]
fn keys_lists_stored_entries() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    cache.store(&CacheKey::new("one", &[]), b"1").unwrap();
    cache.store(&CacheKey::new("two", &[]), b"2").unwrap();
    let mut keys = cache.keys();
    keys.sort();
    assert_eq!(keys.len(), 2);
}

#[test]
fn merge_parents_dedups() {
    let m = ContentCache::merge_parents(&[vec!["a".into(), "b".into()], vec!["b".into(), "c".into()]]);
    assert_eq!(m, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
}

#[test]
fn merge_parents_empty() {
    assert!(ContentCache::merge_parents(&[]).is_empty());
}

#[test]
fn memoize_inserts_once_and_reuses() {
    let mut map: HashMap<String, Vec<u8>> = HashMap::new();
    let key = CacheKey::new("k", &[]);
    let first = ContentCache::memoize(&mut map, &key, b"v1".to_vec());
    let second = ContentCache::memoize(&mut map, &key, b"v2".to_vec());
    assert_eq!(first, second);
    assert_eq!(second, b"v1".to_vec());
    assert_eq!(map.get(&key.hash).unwrap(), &b"v1".to_vec());
}

#[test]
fn distinct_keys_store_distinct_artifacts() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    let k1 = CacheKey::new("RUN a", &[]);
    let k2 = CacheKey::new("RUN b", &[]);
    cache.store(&k1, b"alpha").unwrap();
    cache.store(&k2, b"beta").unwrap();
    assert_eq!(cache.load(&k1).unwrap(), b"alpha");
    assert_eq!(cache.load(&k2).unwrap(), b"beta");
}

#[test]
fn overwrite_existing_key() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    let k = CacheKey::new("RUN x", &[]);
    cache.store(&k, b"old").unwrap();
    cache.store(&k, b"new").unwrap();
    assert_eq!(cache.load(&k).unwrap(), b"new");
}

#[test]
fn parent_hash_chain_is_order_sensitive() {
    let a = CacheKey::new("RUN", &["p1".into(), "p2".into()]);
    let b = CacheKey::new("RUN", &["p2".into(), "p1".into()]);
    assert_ne!(a.hash, b.hash);
}

#[test]
fn empty_cache_has_no_keys() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ContentCache::new(dir.path());
    assert!(cache.keys().is_empty());
}
