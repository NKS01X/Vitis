#![allow(dead_code)]
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

use fuser::{
    BackgroundSession, FileAttr, FileType, Filesystem, MountOption, ReplyAttr, ReplyData,
    ReplyDirectory, ReplyEntry, ReplyOpen, Request,
};
use libc::{EIO, EISDIR, ENOENT};
use reqwest::blocking::Client;
use serde::Deserialize;

pub const TTL: Duration = Duration::from_secs(1);
pub const CHUNK_SIZE: u64 = 65536;

#[derive(Clone)]
pub struct InodeData {
    ino: u64,
    parent: u64,
    name: String,
    file_type: FileType,
    size: u64,
    children: Vec<u64>,
    perm: u16,
    source_url: Option<String>,
}

#[derive(Deserialize)]
struct OciManifest {
    #[serde(default)]
    schema_version: u64,
    config: OciDescriptor,
    layers: Vec<OciDescriptor>,
}

#[derive(Deserialize, Clone)]
struct OciDescriptor {
    media_type: String,
    digest: String,
    size: u64,
}

pub struct VitisFs {
    inodes: Vec<InodeData>,
    cache_dir: PathBuf,
    client: Client,
    chunk_cache: RwLock<HashMap<(u64, u64), Vec<u8>>>,
}

impl VitisFs {
    pub(crate) fn new(inodes: Vec<InodeData>, cache_dir: PathBuf) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build reqwest blocking client");

        Self {
            inodes,
            cache_dir,
            client,
            chunk_cache: RwLock::new(HashMap::new()),
        }
    }

    fn find_child(&self, parent_ino: u64, name: &OsStr) -> Option<&InodeData> {
        let parent = self.inodes.iter().find(|n| n.ino == parent_ino)?;
        let name_str = name.to_str()?;
        for &child_ino in &parent.children {
            if let Some(child) = self.inodes.iter().find(|n| n.ino == child_ino) {
                if child.name == name_str {
                    return Some(child);
                }
            }
        }
        None
    }

    fn build_attr(&self, ino: u64) -> Option<FileAttr> {
        let node = self.inodes.iter().find(|n| n.ino == ino)?;
        Some(FileAttr {
            ino: node.ino,
            size: node.size,
            blocks: node.size.div_ceil(512),
            atime: std::time::UNIX_EPOCH,
            mtime: std::time::UNIX_EPOCH,
            ctime: std::time::UNIX_EPOCH,
            crtime: std::time::UNIX_EPOCH,
            kind: node.file_type,
            perm: node.perm,
            nlink: if node.file_type == FileType::Directory {
                2
            } else {
                1
            },
            uid: 0,
            gid: 0,
            rdev: 0,
            blksize: 4096,
            flags: 0,
        })
    }

    fn get_cache_path(&self, ino: u64) -> PathBuf {
        self.cache_dir.join(ino.to_string())
    }

    fn load_from_cache(&self, ino: u64) -> Option<Vec<u8>> {
        let path = self.get_cache_path(ino);
        if path.exists() {
            std::fs::read(&path).ok()
        } else {
            None
        }
    }

    fn fetch_range(&self, url: &str, start: u64, end: u64) -> Result<Vec<u8>, String> {
        let mut last_error = String::new();
        let mut delay = Duration::from_millis(100);

        for attempt in 0..5 {
            let result = (|| -> Result<Vec<u8>, String> {
                let resp = self
                    .client
                    .get(url)
                    .header("Range", format!("bytes={}-{}", start, end))
                    .send()
                    .map_err(|e| format!("HTTP request failed: {e}"))?;

                let status = resp.status();
                if !status.is_success() && status != 206 {
                    return Err(format!("HTTP status {}", status));
                }

                resp.bytes()
                    .map(|b| b.to_vec())
                    .map_err(|e| format!("reading response body: {e}"))
            })();

            match result {
                Ok(data) => return Ok(data),
                Err(e) => {
                    last_error = e;
                    if attempt < 4 {
                        std::thread::sleep(delay);
                        delay = std::cmp::min(delay * 2, Duration::from_secs(5));
                    }
                }
            }
        }

        Err(last_error)
    }
}

impl Filesystem for VitisFs {
    fn lookup(&mut self, _req: &Request, parent: u64, name: &OsStr, reply: ReplyEntry) {
        match self.find_child(parent, name) {
            Some(node) => match self.build_attr(node.ino) {
                Some(attr) => reply.entry(&TTL, &attr, 0),
                None => reply.error(ENOENT),
            },
            None => reply.error(ENOENT),
        }
    }

    fn getattr(&mut self, _req: &Request, ino: u64, reply: ReplyAttr) {
        match self.build_attr(ino) {
            Some(attr) => reply.attr(&TTL, &attr),
            None => reply.error(ENOENT),
        }
    }

    fn readdir(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        let parent = match self.inodes.iter().find(|n| n.ino == ino) {
            Some(n) if n.file_type == FileType::Directory => n,
            _ => {
                reply.error(ENOENT);
                return;
            }
        };

        let mut entries: Vec<(u64, FileType, String)> = Vec::new();
        entries.push((ino, FileType::Directory, ".".to_string()));
        entries.push((parent.parent, FileType::Directory, "..".to_string()));

        for &child_ino in &parent.children {
            if let Some(child) = self.inodes.iter().find(|n| n.ino == child_ino) {
                entries.push((child.ino, child.file_type, child.name.clone()));
            }
        }

        for (i, (entry_ino, entry_type, entry_name)) in
            entries.iter().enumerate().skip(offset as usize)
        {
            if reply.add(*entry_ino, (i + 1) as i64, *entry_type, entry_name) {
                break;
            }
        }

        reply.ok();
    }

    fn open(&mut self, _req: &Request, ino: u64, _flags: i32, reply: ReplyOpen) {
        if self
            .inodes
            .iter()
            .any(|n| n.ino == ino && n.file_type == FileType::RegularFile)
        {
            reply.opened(0, 0);
        } else {
            reply.error(ENOENT);
        }
    }

    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let node = match self.inodes.iter().find(|n| n.ino == ino) {
            Some(n) => n,
            None => {
                reply.error(ENOENT);
                return;
            }
        };

        if node.file_type != FileType::RegularFile {
            reply.error(EISDIR);
            return;
        }

        let file_size = node.size as i64;
        if offset >= file_size {
            reply.data(&[]);
            return;
        }

        let end = std::cmp::min(offset + size as i64, file_size);

        if let Some(cached) = self.load_from_cache(ino) {
            let start = offset as usize;
            let end_idx = std::cmp::min(end as usize, cached.len());
            if start < end_idx {
                reply.data(&cached[start..end_idx]);
            } else {
                reply.data(&[]);
            }
            return;
        }

        let mut result = Vec::with_capacity((end - offset) as usize);
        let mut current = offset;

        while current < end {
            let chunk_start = (current / CHUNK_SIZE as i64) * CHUNK_SIZE as i64;
            let chunk_end = std::cmp::min(chunk_start + CHUNK_SIZE as i64, file_size);
            let cache_key = (ino, chunk_start as u64);

            let chunk = {
                let cache = self.chunk_cache.read().unwrap();
                cache.get(&cache_key).cloned()
            };

            match chunk {
                Some(data) => {
                    let range_start = (current - chunk_start) as usize;
                    let range_end = std::cmp::min((end - chunk_start) as usize, data.len());
                    if range_start < range_end {
                        result.extend_from_slice(&data[range_start..range_end]);
                        current += (range_end - range_start) as i64;
                    } else {
                        break;
                    }
                }
                None => match &node.source_url {
                    Some(url) => {
                        match self.fetch_range(
                            url,
                            chunk_start as u64,
                            (chunk_end - 1).max(chunk_start) as u64,
                        ) {
                            Ok(data) => {
                                if data.is_empty() {
                                    reply.error(EIO);
                                    return;
                                }
                                self.chunk_cache
                                    .write()
                                    .unwrap()
                                    .insert(cache_key, data.clone());
                                let range_start = (current - chunk_start) as usize;
                                let range_end =
                                    std::cmp::min((end - chunk_start) as usize, data.len());
                                if range_start < range_end {
                                    result.extend_from_slice(&data[range_start..range_end]);
                                    current += (range_end - range_start) as i64;
                                } else {
                                    break;
                                }
                            }
                            Err(_) => {
                                reply.error(EIO);
                                return;
                            }
                        }
                    }
                    None => {
                        let zero_start = (current - chunk_start) as usize;
                        let zero_end =
                            std::cmp::min((end - chunk_start) as usize, CHUNK_SIZE as usize);
                        let zero_len = zero_end.saturating_sub(zero_start);
                        result.extend(std::iter::repeat_n(0u8, zero_len));
                        current = end;
                    }
                },
            }
        }

        reply.data(&result);
    }
}

pub(crate) fn build_oci_tree(
    client: &Client,
    registry_url: &str,
    image: &str,
    tag: &str,
) -> Result<Vec<InodeData>, String> {
    let manifest_url = format!("{}/v2/{}/manifests/{}", registry_url, image, tag);
    let blob_base = format!("{}/v2/{}/blobs/", registry_url, image);

    let resp = client
        .get(&manifest_url)
        .send()
        .map_err(|e| format!("manifest fetch failed: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("manifest fetch returned {}", resp.status()));
    }

    let manifest_size = resp.content_length().unwrap_or(4096);

    let manifest: OciManifest = resp
        .json()
        .map_err(|e| format!("manifest parse failed: {e}"))?;

    let mut inodes = Vec::new();
    let mut next_ino: u64 = 2;

    inodes.push(InodeData {
        ino: 1,
        parent: 1,
        name: String::new(),
        file_type: FileType::Directory,
        size: 4096,
        children: Vec::new(),
        perm: 0o755,
        source_url: None,
    });

    let manifest_ino = next_ino;
    next_ino += 1;
    inodes.push(InodeData {
        ino: manifest_ino,
        parent: 1,
        name: "manifest.json".to_string(),
        file_type: FileType::RegularFile,
        size: manifest_size,
        children: Vec::new(),
        perm: 0o444,
        source_url: Some(manifest_url),
    });
    inodes[0].children.push(manifest_ino);

    let config_ino = next_ino;
    next_ino += 1;
    let config_url = format!("{}{}", blob_base, manifest.config.digest);
    inodes.push(InodeData {
        ino: config_ino,
        parent: 1,
        name: manifest
            .config
            .digest
            .trim_start_matches("sha256:")
            .to_string(),
        file_type: FileType::RegularFile,
        size: manifest.config.size,
        children: Vec::new(),
        perm: 0o444,
        source_url: Some(config_url),
    });
    inodes[0].children.push(config_ino);

    let layers_dir_ino = next_ino;
    next_ino += 1;
    inodes.push(InodeData {
        ino: layers_dir_ino,
        parent: 1,
        name: "layers".to_string(),
        file_type: FileType::Directory,
        size: 4096,
        children: Vec::new(),
        perm: 0o755,
        source_url: None,
    });
    inodes[0].children.push(layers_dir_ino);

    for layer in &manifest.layers {
        let layer_ino = next_ino;
        next_ino += 1;
        let layer_url = format!("{}{}", blob_base, layer.digest);
        let layer_name = layer.digest.trim_start_matches("sha256:").to_string();
        inodes.push(InodeData {
            ino: layer_ino,
            parent: layers_dir_ino,
            name: layer_name,
            file_type: FileType::RegularFile,
            size: layer.size,
            children: Vec::new(),
            perm: 0o444,
            source_url: Some(layer_url),
        });
        let layers_dir = &mut inodes[layers_dir_ino as usize - 1];
        layers_dir.children.push(layer_ino);
    }

    Ok(inodes)
}

async fn background_pull(sources: Vec<(u64, String)>, cache_dir: PathBuf) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap();

    for (ino, url) in &sources {
        let cache_path = cache_dir.join(ino.to_string());
        if cache_path.exists() {
            continue;
        }

        match download_full(&client, url, &cache_path).await {
            Ok(()) => {}
            Err(e) => tracing::warn!("background pull failed for ino {}: {}", ino, e),
        }
    }
}

async fn download_full(client: &reqwest::Client, url: &str, dest: &Path) -> anyhow::Result<()> {
    let resp = client.get(url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("HTTP {}", resp.status());
    }

    let bytes = resp.bytes().await?;
    tokio::fs::write(dest, &bytes).await?;
    Ok(())
}

pub async fn mount_and_pull(
    registry_url: &str,
    image: &str,
    tag: &str,
    mount_point: &str,
    cache_dir: &str,
) -> anyhow::Result<BackgroundSession> {
    let mount_path = PathBuf::from(mount_point);
    tokio::fs::create_dir_all(&mount_path).await?;

    let cache_path = PathBuf::from(cache_dir);
    tokio::fs::create_dir_all(&cache_path).await?;

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow::anyhow!("failed to build HTTP client: {e}"))?;

    let inodes = build_oci_tree(&client, registry_url, image, tag)
        .map_err(|e| anyhow::anyhow!("failed to build OCI tree: {e}"))?;

    let sources: Vec<(u64, String)> = inodes
        .iter()
        .filter_map(|n| n.source_url.as_ref().map(|url| (n.ino, url.clone())))
        .collect();

    let fs = VitisFs::new(inodes, cache_path.clone());

    let session = fuser::spawn_mount2(
        fs,
        &mount_path,
        &[MountOption::RO, MountOption::AutoUnmount],
    )?;

    tokio::spawn(async move {
        background_pull(sources, cache_path).await;
    });

    Ok(session)
}

pub(crate) fn build_mock_tree(_registry_url: &str, _image: &str, _tag: &str) -> Vec<InodeData> {
    vec![
        InodeData {
            ino: 1,
            parent: 1,
            name: String::new(),
            file_type: FileType::Directory,
            size: 4096,
            children: vec![2, 6, 3],
            perm: 0o755,
            source_url: None,
        },
        InodeData {
            ino: 2,
            parent: 1,
            name: "manifest.json".to_string(),
            file_type: FileType::RegularFile,
            size: 4096,
            children: vec![],
            perm: 0o444,
            source_url: Some("mock://manifest".into()),
        },
        InodeData {
            ino: 3,
            parent: 1,
            name: "layers".to_string(),
            file_type: FileType::Directory,
            size: 4096,
            children: vec![4, 5],
            perm: 0o755,
            source_url: None,
        },
        InodeData {
            ino: 4,
            parent: 3,
            name: "sha256:abcdef".to_string(),
            file_type: FileType::RegularFile,
            size: 10 * 1024 * 1024,
            children: vec![],
            perm: 0o444,
            source_url: Some("mock://layer1".into()),
        },
        InodeData {
            ino: 5,
            parent: 3,
            name: "sha256:123456".to_string(),
            file_type: FileType::RegularFile,
            size: 20 * 1024 * 1024,
            children: vec![],
            perm: 0o444,
            source_url: Some("mock://layer2".into()),
        },
        InodeData {
            ino: 6,
            parent: 1,
            name: "hello.txt".to_string(),
            file_type: FileType::RegularFile,
            size: 256,
            children: vec![],
            perm: 0o444,
            source_url: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn make_tree() -> Vec<InodeData> {
        vec![
            InodeData {
                ino: 1,
                parent: 1,
                name: "".to_string(),
                file_type: FileType::Directory,
                size: 4096,
                children: vec![2, 3],
                perm: 0o755,
                source_url: None,
            },
            InodeData {
                ino: 2,
                parent: 1,
                name: "readme.txt".to_string(),
                file_type: FileType::RegularFile,
                size: 100,
                children: vec![],
                perm: 0o444,
                source_url: None,
            },
            InodeData {
                ino: 3,
                parent: 1,
                name: "subdir".to_string(),
                file_type: FileType::Directory,
                size: 4096,
                children: vec![],
                perm: 0o755,
                source_url: None,
            },
        ]
    }

    #[test]
    fn test_inode_lookup() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        let found = fs.find_child(1, OsStr::new("readme.txt"));
        assert!(found.is_some());
        assert_eq!(found.unwrap().ino, 2);
    }

    #[test]
    fn test_missing_child() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        assert!(fs.find_child(1, OsStr::new("nope")).is_none());
    }

    #[test]
    fn test_attr_size() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        let attr = fs.build_attr(2).unwrap();
        assert_eq!(attr.size, 100);
    }

    #[test]
    fn test_root_is_dir() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        let attr = fs.build_attr(1).unwrap();
        assert_eq!(attr.kind, FileType::Directory);
    }

    #[test]
    fn test_regular_file_attr() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        let attr = fs.build_attr(2).unwrap();
        assert_eq!(attr.kind, FileType::RegularFile);
        assert_eq!(attr.nlink, 1);
    }

    #[test]
    fn test_dir_nlink_two() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        let attr = fs.build_attr(3).unwrap();
        assert_eq!(attr.nlink, 2);
    }

    #[test]
    fn test_cache_path() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp/cache"));
        assert_eq!(fs.get_cache_path(2), PathBuf::from("/tmp/cache/2"));
    }

    #[test]
    fn test_load_from_cache_miss() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp/nonexistent_cache_dir_xyz"));
        assert!(fs.load_from_cache(999).is_none());
    }

    #[test]
    fn test_mock_tree_shape() {
        let tree = build_mock_tree("x", "y", "z");
        assert_eq!(tree[0].children, vec![2, 6, 3]);
        assert_eq!(tree.len(), 6);
    }

    #[test]
    fn test_mock_tree_layer_sizes() {
        let tree = build_mock_tree("x", "y", "z");
        assert_eq!(tree[3].size, 10 * 1024 * 1024);
        assert_eq!(tree[4].size, 20 * 1024 * 1024);
    }

    #[test]
    fn test_chunk_size_constant() {
        assert_eq!(CHUNK_SIZE, 65536);
    }

    #[test]
    fn test_build_oci_tree_manifest_error() {
        let client = Client::new();
        let res = build_oci_tree(&client, "http://127.0.0.1:1", "img", "tag");
        assert!(res.is_err());
    }

    #[test]
    fn test_chunk_cache_is_empty_initially() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        let guard = fs.chunk_cache.read().unwrap();
        assert!(guard.is_empty());
    }

    #[test]
    fn test_root_children_present() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        assert!(fs.inodes[0].children.contains(&2));
    }

    #[test]
    fn test_find_child_in_subdir() {
        let tree = vec![
            InodeData {
                ino: 1,
                parent: 1,
                name: "".to_string(),
                file_type: FileType::Directory,
                size: 4096,
                children: vec![3],
                perm: 0o755,
                source_url: None,
            },
            InodeData {
                ino: 3,
                parent: 1,
                name: "subdir".to_string(),
                file_type: FileType::Directory,
                size: 4096,
                children: vec![4],
                perm: 0o755,
                source_url: None,
            },
            InodeData {
                ino: 4,
                parent: 3,
                name: "file.txt".to_string(),
                file_type: FileType::RegularFile,
                size: 10,
                children: vec![],
                perm: 0o444,
                source_url: None,
            },
        ];
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        assert!(fs.find_child(3, OsStr::new("file.txt")).is_some());
        assert!(fs.find_child(3, OsStr::new("missing")).is_none());
    }

    #[test]
    fn test_attr_uid_gid_zero() {
        let tree = make_tree();
        let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
        let attr = fs.build_attr(2).unwrap();
        assert_eq!(attr.uid, 0);
        assert_eq!(attr.gid, 0);
    }

    #[test]
    fn test_ttl_one_second() {
        assert_eq!(TTL, Duration::from_secs(1));
    }
}
