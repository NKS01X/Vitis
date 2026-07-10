// Exclusive tests for the `vitis-export` crate: OCI tarball + manifest.

use std::path::PathBuf;

use vitis_export::{build_manifest, compact_layer, package_snapshot, write_manifest, OciLayer, OciManifest};

#[test]
fn sha256_hex_empty_is_known() {
    assert_eq!(
        vitis_export::sha256_hex(""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

#[test]
fn sha256_hex_is_deterministic() {
    assert_eq!(vitis_export::sha256_hex("hello"), vitis_export::sha256_hex("hello"));
}

#[test]
fn sha256_hex_bytes_matches_string() {
    let s = "payload";
    assert_eq!(vitis_export::sha256_hex(s), vitis_export::sha256_hex_bytes(s.as_bytes()));
}

#[test]
fn sha256_hex_bytes_len() {
    assert_eq!(vitis_export::sha256_hex_bytes(b"x").len(), 64);
}

#[test]
fn compact_layer_creates_gz() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("hello.txt"), "world").unwrap();
    let (path, digest) = compact_layer(src.path(), out.path()).unwrap();
    assert!(path.exists());
    assert_eq!(path.extension().unwrap(), "gz");
    assert_eq!(digest.len(), 64);
}

#[test]
fn compact_layer_empty_dir_ok() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let (path, digest) = compact_layer(src.path(), out.path()).unwrap();
    assert!(path.exists());
    assert!(!digest.is_empty());
}

#[test]
fn compact_layer_missing_source_errors() {
    let out = tempfile::tempdir().unwrap();
    let r = compact_layer(std::path::Path::new("/no/such/dir/xyz"), out.path());
    assert!(r.is_err());
}

#[test]
fn build_manifest_schema_version_two() {
    let m = build_manifest(vec![]);
    assert_eq!(m.schema_version, 2);
}

#[test]
fn build_manifest_records_layers() {
    let m = build_manifest(vec![
        OciLayer { media_type: "x".into(), digest: "abc".into(), size: 10 },
        OciLayer { media_type: "y".into(), digest: "def".into(), size: 20 },
    ]);
    assert_eq!(m.layers.len(), 2);
    assert_eq!(m.layers[0].digest, "abc");
    assert_eq!(m.layers[1].size, 20);
}

#[test]
fn build_manifest_config_media_type() {
    let m = build_manifest(vec![]);
    assert_eq!(m.config.media_type, "application/vnd.oci.image.config.v1+json");
}

#[test]
fn build_manifest_config_digest_is_empty_object() {
    let m = build_manifest(vec![]);
    // digest of the empty OCI config object `{}`
    assert!(m.config.digest.starts_with("44136fa355b3678a"));
}

#[test]
fn build_manifest_media_type() {
    let m = build_manifest(vec![]);
    assert_eq!(m.media_type, "application/vnd.oci.image.manifest.v1+json");
}

#[test]
fn write_manifest_roundtrip() {
    let dir = tempfile::tempdir().unwrap();
    let m = build_manifest(vec![]);
    let (path, digest) = write_manifest(&m, dir.path()).unwrap();
    let contents = std::fs::read_to_string(&path).unwrap();
    let parsed: OciManifest = serde_json::from_str(&contents).unwrap();
    assert_eq!(parsed.schema_version, 2);
    assert_eq!(digest.len(), 64);
}

#[test]
fn write_manifest_writes_digest_file() {
    let dir = tempfile::tempdir().unwrap();
    let m = build_manifest(vec![]);
    write_manifest(&m, dir.path()).unwrap();
    assert!(dir.path().join("digest").exists());
}

#[test]
fn package_snapshot_single_layer() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("f.txt"), "data").unwrap();
    let (_path, digest) = package_snapshot(&[("task0".into(), src.path().to_path_buf())], out.path()).unwrap();
    assert_eq!(digest.len(), 64);
}

#[test]
fn package_snapshot_skips_missing_dirs() {
    let out = tempfile::tempdir().unwrap();
    let r = package_snapshot(&[("missing".into(), PathBuf::from("/no/such/xyz"))], out.path());
    assert!(r.is_ok());
    let (_path, digest) = r.unwrap();
    assert_eq!(digest.len(), 64);
}

#[test]
fn layer_media_type_constant() {
    let m = build_manifest(vec![OciLayer {
        media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
        digest: "d".into(),
        size: 1,
    }]);
    assert_eq!(m.layers[0].media_type, "application/vnd.oci.image.layer.v1.tar+gzip");
}

#[test]
fn layer_digest_preserved_verbatim() {
    let m = build_manifest(vec![OciLayer {
        media_type: "x".into(),
        digest: "sha256:deadbeef".into(),
        size: 1,
    }]);
    assert_eq!(m.layers[0].digest, "sha256:deadbeef");
}

#[test]
fn gzip_is_smaller_than_plaintext() {
    let src = tempfile::tempdir().unwrap();
    let big = "a".repeat(10000);
    std::fs::write(src.path().join("big.txt"), &big).unwrap();
    let out = tempfile::tempdir().unwrap();
    let (path, _digest) = compact_layer(src.path(), out.path()).unwrap();
    let compressed = std::fs::metadata(&path).unwrap().len();
    assert!(compressed < 10000);
}

#[test]
fn multiple_layers_total_size() {
    let m = build_manifest(vec![
        OciLayer { media_type: "x".into(), digest: "a".into(), size: 5 },
        OciLayer { media_type: "y".into(), digest: "b".into(), size: 7 },
    ]);
    let total: u64 = m.layers.iter().map(|l| l.size).sum();
    assert_eq!(total, 12);
}

#[test]
fn manifest_serializes_to_json() {
    let m = build_manifest(vec![OciLayer { media_type: "x".into(), digest: "a".into(), size: 1 }]);
    let json = serde_json::to_string(&m).unwrap();
    assert!(json.contains("schema_version"));
    assert!(json.contains("layers"));
    assert!(json.contains("config"));
}

#[test]
fn package_snapshot_creates_layers_dir() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("f.txt"), "data").unwrap();
    package_snapshot(&[("task0".into(), src.path().to_path_buf())], out.path()).unwrap();
    assert!(out.path().join("layers").exists());
}
