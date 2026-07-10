// Exclusive tests for the `vitis-storage` crate: the FUSE lazy-pull filesystem.
//
// These tests require the `fuse` feature (and libfuse3-dev on the host) because
// the crate links against the system FUSE library. Run with:
//     cd tests/storage && cargo test --features fuse
//
// Without the feature, this file compiles to nothing so the rest of the suite
// stays green on machines without FUSE headers installed.

#![cfg(feature = "fuse")]

use std::ffi::OsStr;
use std::path::PathBuf;
use std::time::Duration;

use vitis_storage::{build_mock_tree, build_oci_tree, CHUNK_SIZE, TTL, VitisFs};

#[test]
fn inode_lookup_by_name() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    let found = fs.find_child(1, OsStr::new("manifest.json"));
    assert!(found.is_some());
    assert_eq!(found.unwrap().ino, 2);
}

#[test]
fn lookup_missing_child_is_none() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert!(fs.find_child(1, OsStr::new("nope")).is_none());
}

#[test]
fn lookup_child_in_layers_dir() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert!(fs.find_child(3, OsStr::new("sha256:abcdef")).is_some());
    assert!(fs.find_child(3, OsStr::new("missing")).is_none());
}

#[test]
fn attr_size_matches_node() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert_eq!(fs.build_attr(2).unwrap().size, 4096);
    assert_eq!(fs.build_attr(4).unwrap().size, 10 * 1024 * 1024);
}

#[test]
fn root_is_directory() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert_eq!(fs.build_attr(1).unwrap().kind, fuser::FileType::Directory);
}

#[test]
fn layers_dir_is_directory() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert_eq!(fs.build_attr(3).unwrap().kind, fuser::FileType::Directory);
}

#[test]
fn regular_file_attr() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    let attr = fs.build_attr(4).unwrap();
    assert_eq!(attr.kind, fuser::FileType::RegularFile);
    assert_eq!(attr.nlink, 1);
}

#[test]
fn dir_nlink_is_two() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert_eq!(fs.build_attr(3).unwrap().nlink, 2);
}

#[test]
fn cache_path_uses_ino() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp/cache"));
    assert_eq!(fs.get_cache_path(2), PathBuf::from("/tmp/cache/2"));
}

#[test]
fn load_from_cache_miss() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp/no_such_cache_dir"));
    assert!(fs.load_from_cache(999).is_none());
}

#[test]
fn mock_tree_shape() {
    let tree = build_mock_tree("x", "y", "z");
    assert_eq!(tree[0].children, vec![2, 6, 3]);
    assert_eq!(tree.len(), 6);
}

#[test]
fn mock_tree_layer_sizes() {
    let tree = build_mock_tree("x", "y", "z");
    assert_eq!(tree[3].size, 10 * 1024 * 1024);
    assert_eq!(tree[4].size, 20 * 1024 * 1024);
}

#[test]
fn chunk_size_constant() {
    assert_eq!(CHUNK_SIZE, 65536);
}

#[test]
fn ttl_one_second() {
    assert_eq!(TTL, Duration::from_secs(1));
}

#[test]
fn build_oci_tree_unreachable_errors() {
    let client = reqwest::blocking::Client::new();
    let res = build_oci_tree(&client, "http://127.0.0.1:1", "img", "tag");
    assert!(res.is_err());
}

#[test]
fn chunk_cache_empty_initially() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert!(fs.chunk_cache.read().unwrap().is_empty());
}

#[test]
fn attr_uid_gid_zero() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    let attr = fs.build_attr(2).unwrap();
    assert_eq!(attr.uid, 0);
    assert_eq!(attr.gid, 0);
}

#[test]
fn root_contains_expected_children() {
    let tree = build_mock_tree("x", "y", "z");
    let fs = VitisFs::new(tree, PathBuf::from("/tmp"));
    assert!(fs.inodes[0].children.contains(&2));
    assert!(fs.inodes[0].children.contains(&3));
    assert!(fs.inodes[0].children.contains(&6));
}
