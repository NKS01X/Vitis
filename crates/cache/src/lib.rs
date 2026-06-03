use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffDelta {
    pub relative_path: String,
    pub kind: DiffKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffKind {
    Added,
    Modified,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheKey {
    pub hash: String,
    pub instruction: String,
    pub parent_hashes: Vec<String>,
    pub copy_hashes: Vec<String>,
}

impl CacheKey {
    pub fn new(instruction: &str, parent_hashes: &[String], copy_targets: &[Vec<u8>]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(instruction.as_bytes());
        for p in parent_hashes {
            hasher.update(p.as_bytes());
        }
        let mut copy_hashes: Vec<String> = Vec::new();
        for target in copy_targets {
            let mut ch = Sha256::new();
            ch.update(target);
            let copy_hash = hex::encode(ch.finalize());
            hasher.update(copy_hash.as_bytes());
            copy_hashes.push(copy_hash);
        }
        let hash = hex::encode(hasher.finalize());
        Self {
            hash,
            instruction: instruction.to_string(),
            parent_hashes: parent_hashes.to_vec(),
            copy_hashes,
        }
    }
}

pub struct ContentCache {
    root: PathBuf,
}

impl ContentCache {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{}.bin", key))
    }

    pub fn hit(&self, key: &CacheKey) -> bool {
        self.path_for(&key.hash).exists()
    }

    pub fn store(&self, key: &CacheKey, data: &[u8]) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        std::fs::write(self.path_for(&key.hash), data)
    }

    pub fn load(&self, key: &CacheKey) -> Option<Vec<u8>> {
        std::fs::read(self.path_for(&key.hash)).ok()
    }

    pub fn diff(a: &[u8], b: &[u8]) -> Vec<(usize, usize)> {
        let mut ranges = Vec::new();
        let mut start: Option<usize> = None;
        let len = a.len().max(b.len());
        for i in 0..len {
            let ba = a.get(i).copied().unwrap_or(0);
            let bb = b.get(i).copied().unwrap_or(0);
            if ba != bb {
                if start.is_none() {
                    start = Some(i);
                }
            } else if let Some(s) = start.take() {
                ranges.push((s, i));
            }
        }
        if let Some(s) = start.take() {
            ranges.push((s, len));
        }
        ranges
    }

    pub fn diff_size(a: &[u8], b: &[u8]) -> usize {
        Self::diff(a, b).iter().map(|(s, e)| e - s).sum()
    }

    pub fn keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for e in entries.flatten() {
                let name = e.file_name();
                if let Some(s) = name.to_str() {
                    if let Some(stripped) = s.strip_suffix(".bin") {
                        out.push(stripped.to_string());
                    }
                }
            }
        }
        out
    }

    pub fn merge_parents(parents: &[Vec<String>]) -> Vec<String> {
        let mut merged: Vec<String> = Vec::new();
        for p in parents {
            for h in p {
                if !merged.contains(h) {
                    merged.push(h.clone());
                }
            }
        }
        merged
    }

    pub fn memoize(map: &mut HashMap<String, Vec<u8>>, key: &CacheKey, data: Vec<u8>) -> Vec<u8> {
        map.entry(key.hash.clone()).or_insert(data.clone());
        map.get(&key.hash).cloned().unwrap()
    }

    pub fn snapshot_diff(
        base: &Path,
        running: &Path,
        dest: &Path,
    ) -> std::io::Result<Vec<DiffDelta>> {
        let mut deltas = Vec::new();
        fs::create_dir_all(dest)?;
        collect_deltas(base, running, running, dest, &mut deltas)?;
        Ok(deltas)
    }
}

impl Default for ContentCache {
    fn default() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        Self {
            root: PathBuf::from(home).join(".vitis").join("cache"),
        }
    }
}

fn collect_deltas(
    base: &Path,
    running: &Path,
    scan_dir: &Path,
    dest: &Path,
    deltas: &mut Vec<DiffDelta>,
) -> std::io::Result<()> {
    for entry in fs::read_dir(scan_dir)? {
        let entry = entry?;
        let path = entry.path();
        let relative = path.strip_prefix(running).unwrap_or(&path);
        let relative_str = relative.to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            collect_deltas(base, running, &path, dest, deltas)?;
        } else {
            let running_bytes = fs::read(&path)?;
            let base_path = base.join(relative);
            let kind = if base_path.exists() {
                let base_bytes = fs::read(&base_path)?;
                if base_bytes == running_bytes {
                    continue;
                }
                DiffKind::Modified
            } else {
                DiffKind::Added
            };
            let dest_path = dest.join(relative);
            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&dest_path, &running_bytes)?;
            deltas.push(DiffDelta {
                relative_path: relative_str,
                kind,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_deterministic() {
        let a = CacheKey::new("RUN make", &["x".into()], &[]);
        let b = CacheKey::new("RUN make", &["x".into()], &[]);
        assert_eq!(a, b);
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn key_changes_with_instruction() {
        let a = CacheKey::new("RUN make", &["x".into()], &[]);
        let b = CacheKey::new("RUN build", &["x".into()], &[]);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn key_changes_with_parents() {
        let a = CacheKey::new("RUN make", &["x".into()], &[]);
        let b = CacheKey::new("RUN make", &["y".into()], &[]);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn store_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ContentCache::new(dir.path());
        let key = CacheKey::new("RUN echo hi", &[], &[]);
        cache.store(&key, b"artifact").unwrap();
        assert!(cache.hit(&key));
        assert_eq!(cache.load(&key).unwrap(), b"artifact");
    }

    #[test]
    fn miss_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ContentCache::new(dir.path());
        assert!(!cache.hit(&CacheKey::new("nope", &[], &[])));
        assert!(cache.load(&CacheKey::new("nope", &[], &[])).is_none());
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
    fn diff_size_counts_bytes() {
        assert_eq!(ContentCache::diff_size(b"aaaa", b"bbbb"), 4);
    }

    #[test]
    fn diff_different_lengths() {
        let d = ContentCache::diff(b"abc", b"abcd");
        assert_eq!(d, vec![(3, 4)]);
    }

    #[test]
    fn merge_parents_dedups() {
        let m = ContentCache::merge_parents(&[
            vec!["a".into(), "b".into()],
            vec!["b".into(), "c".into()],
        ]);
        assert_eq!(m, vec!["a", "b", "c"]);
    }

    #[test]
    fn merge_parents_empty() {
        assert!(ContentCache::merge_parents(&[]).is_empty());
    }

    #[test]
    fn keys_lists_stored() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ContentCache::new(dir.path());
        cache.store(&CacheKey::new("one", &[], &[]), b"1").unwrap();
        cache.store(&CacheKey::new("two", &[], &[]), b"2").unwrap();
        let mut keys = cache.keys();
        keys.sort();
        assert_eq!(keys.len(), 2);
    }

    #[test]
    fn memoize_inserts_once() {
        let mut map = HashMap::new();
        let key = CacheKey::new("k", &[], &[]);
        let first = ContentCache::memoize(&mut map, &key, b"v1".to_vec());
        let second = ContentCache::memoize(&mut map, &key, b"v2".to_vec());
        assert_eq!(first, second);
        assert_eq!(second, b"v1".to_vec());
    }

    #[test]
    fn hash_is_hex_string() {
        let key = CacheKey::new("x", &[], &[]);
        assert_eq!(key.hash.len(), 64);
        assert!(key.hash.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn store_creates_dir() {
        let dir = tempfile::tempdir().unwrap().into_path().join("nested/deep");
        let cache = ContentCache::new(&dir);
        cache.store(&CacheKey::new("k", &[], &[]), b"x").unwrap();
        assert!(dir.exists());
    }

    #[test]
    fn multi_range_diff() {
        let d = ContentCache::diff(b"AAAA BBBB CCCC", b"XXXX BBBB YYYY");
        assert_eq!(d, vec![(0, 4), (10, 14)]);
    }

    #[test]
    fn parent_hashes_recorded() {
        let key = CacheKey::new("RUN", &["p1".into(), "p2".into()], &[]);
        assert_eq!(key.parent_hashes, vec!["p1", "p2"]);
    }

    #[test]
    fn empty_instruction_key() {
        let key = CacheKey::new("", &[], &[]);
        assert!(!key.hash.is_empty());
    }

    #[test]
    fn diff_trailing_only() {
        let d = ContentCache::diff(b"fixed", b"fixedextra");
        assert_eq!(d, vec![(5, 10)]);
    }

    #[test]
    fn key_changes_with_copy_content() {
        let a = CacheKey::new("RUN make", &["x".into()], &[b"file_a".to_vec()]);
        let b = CacheKey::new("RUN make", &["x".into()], &[b"file_b".to_vec()]);
        assert_ne!(a.hash, b.hash);
    }

    #[test]
    fn key_identical_with_same_copy_content() {
        let a = CacheKey::new("RUN make", &["x".into()], &[b"a".to_vec()]);
        let b = CacheKey::new("RUN make", &["x".into()], &[b"a".to_vec()]);
        assert_eq!(a, b);
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn key_without_copy_equals_empty_copy() {
        let a = CacheKey::new("RUN make", &[], &[]);
        let b = CacheKey::new("RUN make", &[], &[]);
        assert_eq!(a.hash, b.hash);
    }

    #[test]
    fn default_root_is_vitis_cache_dir() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let expected = PathBuf::from(home).join(".vitis").join("cache");
        let cache = ContentCache::default();
        assert_eq!(cache.root, expected);
    }

    #[test]
    fn snapshot_diff_detects_added_file() {
        let base = tempfile::tempdir().unwrap();
        let running = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(running.path().join("new.txt"), b"hello").unwrap();
        std::fs::write(base.path().join("existing.txt"), b"unchanged").unwrap();
        std::fs::write(running.path().join("existing.txt"), b"unchanged").unwrap();
        let deltas = ContentCache::snapshot_diff(base.path(), running.path(), dest.path()).unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].relative_path, "new.txt");
        assert_eq!(deltas[0].kind, DiffKind::Added);
    }

    #[test]
    fn snapshot_diff_detects_modified_file() {
        let base = tempfile::tempdir().unwrap();
        let running = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(base.path().join("changed.txt"), b"old").unwrap();
        std::fs::write(running.path().join("changed.txt"), b"new").unwrap();
        let deltas = ContentCache::snapshot_diff(base.path(), running.path(), dest.path()).unwrap();
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].relative_path, "changed.txt");
        assert_eq!(deltas[0].kind, DiffKind::Modified);
    }

    #[test]
    fn snapshot_diff_ignores_unchanged_files() {
        let base = tempfile::tempdir().unwrap();
        let running = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        std::fs::write(base.path().join("same.txt"), b"data").unwrap();
        std::fs::write(running.path().join("same.txt"), b"data").unwrap();
        let deltas = ContentCache::snapshot_diff(base.path(), running.path(), dest.path()).unwrap();
        assert!(deltas.is_empty());
    }

    #[test]
    fn snapshot_diff_writes_modified_files_to_dest() {
        let base = tempfile::tempdir().unwrap();
        let running = tempfile::tempdir().unwrap();
        let dest = tempfile::tempdir().unwrap();
        let dest_hash = "aaaa";
        let hash_dir = dest.path().join(dest_hash);
        std::fs::write(base.path().join("old.txt"), b"old").unwrap();
        std::fs::write(running.path().join("old.txt"), b"old_modified").unwrap();
        std::fs::write(running.path().join("new.txt"), b"brand_new").unwrap();
        ContentCache::snapshot_diff(base.path(), running.path(), &hash_dir).unwrap();
        assert!(hash_dir.join("old.txt").exists());
        assert!(hash_dir.join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(hash_dir.join("old.txt")).unwrap(),
            "old_modified"
        );
    }
}
