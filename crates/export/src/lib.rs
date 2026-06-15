use std::io::{self, Write};
use std::path::{Path, PathBuf};

use flate2::write::GzEncoder;
use flate2::Compression;
use sha2::{Digest, Sha256};

pub fn sha256_hex(data: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn sha256_hex_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

pub fn compact_layer(source_dir: &Path, output_dir: &Path) -> io::Result<(PathBuf, String)> {
    std::fs::create_dir_all(output_dir)?;

    let mut tar_bytes = Vec::new();
    {
        let mut builder = tar::Builder::new(&mut tar_bytes);
        builder.append_dir_all(".", source_dir)?;
        builder.finish()?;
    }

    let mut hasher = Sha256::new();
    hasher.update(&tar_bytes);
    let digest = hex::encode(hasher.finalize());

    let out_path = output_dir.join(format!("{}.tar.gz", digest));
    let file = std::fs::File::create(&out_path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(&tar_bytes)?;
    encoder.finish()?;

    Ok((out_path, digest))
}

#[derive(Debug, serde::Serialize)]
pub struct OciLayer {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OciManifest {
    pub schema_version: u64,
    pub media_type: String,
    #[serde(rename = "config")]
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
}

#[derive(Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct OciDescriptor {
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OciConfig {
    pub architecture: String,
    pub os: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rootfs: Option<OciRootFs>,
    pub config: OciConfigRoot,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct OciRootFs {
    #[serde(rename = "type")]
    pub type_: String,
    pub diff_ids: Vec<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct OciConfigRoot {
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cmd: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entrypoint: Option<Vec<String>>,
}

pub fn build_config(
    env: Vec<String>,
    working_dir: Option<String>,
    cmd: Option<Vec<String>>,
    entrypoint: Option<Vec<String>>,
) -> OciConfig {
    OciConfig {
        architecture: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        rootfs: Some(OciRootFs {
            type_: "layers".into(),
            diff_ids: Vec::new(),
        }),
        config: OciConfigRoot {
            env,
            working_dir,
            cmd,
            entrypoint,
        },
    }
}

pub fn write_config(config: &OciConfig, output_dir: &Path) -> io::Result<(PathBuf, String)> {
    std::fs::create_dir_all(output_dir)?;
    let json = serde_json::to_string_pretty(config).map_err(io::Error::other)?;
    let digest = sha256_hex(&json);
    let path = output_dir.join(format!("{}.json", digest));
    std::fs::write(&path, &json)?;
    Ok((path, digest))
}

pub fn build_manifest(layers: Vec<OciLayer>, config_digest: &str, config_size: u64) -> OciManifest {
    OciManifest {
        schema_version: 2,
        media_type: "application/vnd.oci.image.manifest.v1+json".to_string(),
        config: OciDescriptor {
            media_type: "application/vnd.oci.image.config.v1+json".to_string(),
            digest: config_digest.to_string(),
            size: config_size,
        },
        layers: layers
            .into_iter()
            .map(|l| OciDescriptor {
                media_type: l.media_type,
                digest: l.digest,
                size: l.size,
            })
            .collect(),
    }
}

pub fn write_manifest(manifest: &OciManifest, output_dir: &Path) -> io::Result<(PathBuf, String)> {
    std::fs::create_dir_all(output_dir)?;
    let json = serde_json::to_string_pretty(manifest).map_err(io::Error::other)?;
    let digest = sha256_hex(&json);
    let path = output_dir.join(format!("{}.json", digest));
    std::fs::write(&path, &json)?;
    std::fs::write(output_dir.join("digest"), &digest)?;
    Ok((path, digest))
}

pub fn package_snapshot(
    task_dirs: &[(String, PathBuf)],
    output_dir: &Path,
) -> io::Result<(PathBuf, String)> {
    let mut layers = Vec::new();
    for (name, dir) in task_dirs {
        if !dir.exists() {
            continue;
        }
        let (layer_path, digest) = compact_layer(dir, &output_dir.join("layers"))?;
        let size = std::fs::metadata(&layer_path)?.len();
        layers.push(OciLayer {
            media_type: "application/vnd.oci.image.layer.v1.tar+gzip".to_string(),
            digest,
            size,
        });
        let _ = name;
    }
    let config = build_config(vec![], None, None, None);
    let config_json = serde_json::to_string_pretty(&config).map_err(io::Error::other)?;
    let config_digest = sha256_hex(&config_json);
    let config_size = config_json.len() as u64;
    let manifest = build_manifest(layers, &config_digest, config_size);
    write_manifest(&manifest, output_dir)
}

pub fn package_image(
    config: &OciConfig,
    manifest: &OciManifest,
    layer_blobs: &[(String, Vec<u8>)],
    output_tar: &Path,
) -> io::Result<()> {
    let file = std::fs::File::create(output_tar)?;
    let mut builder = tar::Builder::new(file);

    let oci_layout = serde_json::json!({"imageLayoutVersion": "1.0.0"});
    let oci_layout_bytes = serde_json::to_vec_pretty(&oci_layout).map_err(io::Error::other)?;
    let mut header = tar::Header::new_gnu();
    header.set_path("oci-layout").map_err(io::Error::other)?;
    header.set_size(oci_layout_bytes.len() as u64);
    header.set_mode(0o444);
    header.set_cksum();
    builder.append(&header, &oci_layout_bytes[..])?;

    let config_json = serde_json::to_vec_pretty(config).map_err(io::Error::other)?;
    let config_digest = sha256_hex_bytes(&config_json);
    let mut header = tar::Header::new_gnu();
    header
        .set_path(format!("blobs/sha256/{}", config_digest))
        .map_err(io::Error::other)?;
    header.set_size(config_json.len() as u64);
    header.set_mode(0o444);
    header.set_cksum();
    builder.append(&header, &config_json[..])?;

    let manifest_json = serde_json::to_vec_pretty(manifest).map_err(io::Error::other)?;
    let manifest_digest = sha256_hex_bytes(&manifest_json);
    let mut header = tar::Header::new_gnu();
    header
        .set_path(format!("blobs/sha256/{}", manifest_digest))
        .map_err(io::Error::other)?;
    header.set_size(manifest_json.len() as u64);
    header.set_mode(0o444);
    header.set_cksum();
    builder.append(&header, &manifest_json[..])?;

    for (layer_digest, layer_data) in layer_blobs {
        let mut header = tar::Header::new_gnu();
        header
            .set_path(format!("blobs/sha256/{}", layer_digest))
            .map_err(io::Error::other)?;
        header.set_size(layer_data.len() as u64);
        header.set_mode(0o444);
        header.set_cksum();
        builder.append(&header, &layer_data[..])?;
    }

    let index_json = serde_json::json!({
        "schemaVersion": 2,
        "manifests": [{
            "mediaType": "application/vnd.oci.image.manifest.v1+json",
            "digest": format!("sha256:{}", manifest_digest),
            "size": manifest_json.len()
        }]
    });
    let index_bytes = serde_json::to_vec_pretty(&index_json).map_err(io::Error::other)?;
    let mut header = tar::Header::new_gnu();
    header.set_path("index.json").map_err(io::Error::other)?;
    header.set_size(index_bytes.len() as u64);
    header.set_mode(0o444);
    header.set_cksum();
    builder.append(&header, &index_bytes[..])?;

    builder.finish()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_hex() {
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn test_compact_layer_creates_tar_gz() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("hello.txt"), "world").unwrap();
        let result = compact_layer(src.path(), out.path());
        assert!(result.is_ok());
        let (path, digest) = result.unwrap();
        assert!(path.exists());
        assert!(path.extension().unwrap() == "gz");
        assert!(!digest.is_empty());
        assert!(digest.len() == 64);
    }

    #[test]
    fn test_compact_layer_empty_dir() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let result = compact_layer(src.path(), out.path());
        assert!(result.is_ok());
        let (path, digest) = result.unwrap();
        assert!(path.exists());
        assert!(!digest.is_empty());
    }

    #[test]
    fn test_compact_layer_nonexistent_source() {
        let out = tempfile::tempdir().unwrap();
        let result = compact_layer(Path::new("/no/such/dir/xyz"), out.path());
        assert!(result.is_err());
    }

    #[test]
    fn test_build_manifest() {
        let manifest = build_manifest(
            vec![
                OciLayer {
                    media_type: "x".into(),
                    digest: "abc".into(),
                    size: 10,
                },
                OciLayer {
                    media_type: "y".into(),
                    digest: "def".into(),
                    size: 20,
                },
            ],
            "config_digest",
            100,
        );
        assert_eq!(manifest.schema_version, 2);
        assert_eq!(manifest.layers.len(), 2);
        assert_eq!(manifest.layers[0].digest, "abc");
        assert_eq!(manifest.config.digest, "config_digest");
    }

    #[test]
    fn test_write_manifest_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = build_manifest(vec![], "cd", 10);
        let (path, digest) = write_manifest(&manifest, dir.path()).unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: OciManifest = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed.schema_version, 2);
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn test_package_snapshot_single_layer() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("f.txt"), "data").unwrap();
        let result = package_snapshot(&[("task0".into(), src.path().to_path_buf())], out.path());
        assert!(result.is_ok());
        let (_, digest) = result.unwrap();
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn test_package_snapshot_skips_missing() {
        let out = tempfile::tempdir().unwrap();
        let result = package_snapshot(
            &[("missing".into(), PathBuf::from("/no/such/xyz"))],
            out.path(),
        );
        assert!(result.is_ok());
        let (_, digest) = result.unwrap();
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn test_layer_media_type() {
        let manifest = build_manifest(
            vec![OciLayer {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
                digest: "d".into(),
                size: 1,
            }],
            "cd",
            10,
        );
        assert_eq!(
            manifest.layers[0].media_type,
            "application/vnd.oci.image.layer.v1.tar+gzip"
        );
    }

    #[test]
    fn test_sha256_hex_bytes() {
        let h = sha256_hex_bytes(b"hello");
        assert_eq!(h.len(), 64);
    }

    #[test]
    fn test_gzip_is_smaller_or_equal() {
        let src = tempfile::tempdir().unwrap();
        let big = "a".repeat(10000);
        std::fs::write(src.path().join("big.txt"), &big).unwrap();
        let out = tempfile::tempdir().unwrap();
        let (path, _) = compact_layer(src.path(), out.path()).unwrap();
        let compressed = std::fs::metadata(&path).unwrap().len();
        assert!(compressed < 10000);
    }

    #[test]
    fn test_multiple_layers_total_size() {
        let manifest = build_manifest(
            vec![
                OciLayer {
                    media_type: "x".into(),
                    digest: "a".into(),
                    size: 5,
                },
                OciLayer {
                    media_type: "y".into(),
                    digest: "b".into(),
                    size: 7,
                },
            ],
            "cd",
            10,
        );
        let total: u64 = manifest.layers.iter().map(|l| l.size).sum();
        assert_eq!(total, 12);
    }

    #[test]
    fn test_digest_file_written() {
        let dir = tempfile::tempdir().unwrap();
        let m = build_manifest(vec![], "cd", 10);
        write_manifest(&m, dir.path()).unwrap();
        assert!(dir.path().join("digest").exists());
    }

    #[test]
    fn test_config_media_type() {
        let m = build_manifest(vec![], "cd", 10);
        assert_eq!(
            m.config.media_type,
            "application/vnd.oci.image.config.v1+json"
        );
    }

    #[test]
    fn test_layer_digest_preserved() {
        let m = build_manifest(
            vec![OciLayer {
                media_type: "x".into(),
                digest: "sha256:deadbeef".into(),
                size: 1,
            }],
            "cd",
            10,
        );
        assert_eq!(m.layers[0].digest, "sha256:deadbeef");
    }

    #[test]
    fn test_build_config_fields() {
        let config = build_config(
            vec!["PATH=/usr/bin".into()],
            Some("/app".into()),
            Some(vec!["bash".into()]),
            Some(vec!["/init".into()]),
        );
        assert_eq!(config.config.env, vec!["PATH=/usr/bin"]);
        assert_eq!(config.config.working_dir, Some("/app".into()));
        assert_eq!(config.config.cmd, Some(vec!["bash".into()]));
        assert_eq!(config.config.entrypoint, Some(vec!["/init".into()]));
        assert!(config.rootfs.is_some());
    }

    #[test]
    fn test_write_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let config = build_config(vec![], None, None, None);
        let (path, digest) = write_config(&config, dir.path()).unwrap();
        assert!(path.exists());
        assert_eq!(digest.len(), 64);
    }

    #[test]
    fn test_package_image_contains_oci_layout() {
        let out = tempfile::tempdir().unwrap();
        let output_tar = out.path().join("output.tar");
        let config = build_config(vec![], None, None, None);
        let config_json = serde_json::to_vec_pretty(&config).unwrap();
        let config_digest = sha256_hex_bytes(&config_json);
        let layers = vec![(sha256_hex_bytes(b"layer_data"), b"layer_data".to_vec())];
        let manifest = build_manifest(
            vec![OciLayer {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
                digest: layers[0].0.clone(),
                size: layers[0].1.len() as u64,
            }],
            &config_digest,
            config_json.len() as u64,
        );
        package_image(&config, &manifest, &layers, &output_tar).unwrap();
        assert!(output_tar.exists());
    }

    #[test]
    fn test_package_image_entries() {
        let out = tempfile::tempdir().unwrap();
        let output_tar = out.path().join("output.tar");
        let config = build_config(vec![], None, None, None);
        let config_json = serde_json::to_vec_pretty(&config).unwrap();
        let config_digest = sha256_hex_bytes(&config_json);
        let layers = vec![(sha256_hex_bytes(b"layer_data"), b"layer_data".to_vec())];
        let manifest = build_manifest(
            vec![OciLayer {
                media_type: "application/vnd.oci.image.layer.v1.tar+gzip".into(),
                digest: layers[0].0.clone(),
                size: layers[0].1.len() as u64,
            }],
            &config_digest,
            config_json.len() as u64,
        );
        package_image(&config, &manifest, &layers, &output_tar).unwrap();
        let archive = std::fs::read(&output_tar).unwrap();
        let mut ar = tar::Archive::new(&archive[..]);
        let entries: Vec<String> = ar
            .entries()
            .unwrap()
            .filter_map(|e| {
                e.ok()
                    .map(|e| e.path().unwrap().to_string_lossy().to_string())
            })
            .collect();
        assert!(entries.contains(&"oci-layout".to_string()));
        assert!(entries.contains(&"index.json".to_string()));
        assert!(entries.iter().any(|p| p.starts_with("blobs/sha256/")));
    }
}
