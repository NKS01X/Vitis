// Exclusive tests for the `vitis-registry` crate: OCI registry client.

use vitis_registry::{OciDescriptor, OciManifest, RegistryClient, RegistryError};

fn client() -> RegistryClient {
    RegistryClient::new("https://registry.example.com", "library/ubuntu")
}

#[test]
fn base_url_trims_trailing_slash() {
    let c = RegistryClient::new("https://x.com/", "img");
    assert_eq!(c.base_url(), "https://x.com");
}

#[test]
fn manifest_url_shape() {
    let c = client();
    assert_eq!(
        c.manifest_url("24.04"),
        "https://registry.example.com/v2/library/ubuntu/manifests/24.04"
    );
}

#[test]
fn blob_url_shape() {
    let c = client();
    assert_eq!(
        c.blob_url("sha256:abc"),
        "https://registry.example.com/v2/library/ubuntu/blobs/sha256:abc"
    );
}

#[test]
fn error_display_contains_context() {
    let e = RegistryError::ManifestNotFound("img".into(), "t".into());
    assert!(e.to_string().contains("img"));
    assert!(e.to_string().contains("t"));
}

#[test]
fn retry_exhausted_display() {
    let e = RegistryError::RetryExhausted(5, "boom".into());
    assert!(e.to_string().contains("5"));
    assert!(e.to_string().contains("boom"));
}

#[test]
fn blob_not_found_display() {
    let e = RegistryError::BlobNotFound("sha256:x".into());
    assert!(e.to_string().contains("sha256:x"));
}

#[test]
fn http_error_display() {
    let e = RegistryError::Http("connection reset".into());
    assert!(e.to_string().contains("connection reset"));
}

#[test]
fn parse_manifest_json() {
    let json = r#"{
        "schemaVersion": 2,
        "config": {"mediaType": "t", "digest": "sha256:c", "size": 1},
        "layers": [{"mediaType": "l", "digest": "sha256:l1", "size": 2}]
    }"#;
    let m: OciManifest = serde_json::from_str(json).unwrap();
    assert_eq!(m.schema_version, 2);
    assert_eq!(m.layers.len(), 1);
    assert_eq!(m.layers[0].digest, "sha256:l1");
}

#[test]
fn parse_manifest_default_schema_version() {
    let json = r#"{
        "config": {"mediaType": "t", "digest": "sha256:c", "size": 1},
        "layers": []
    }"#;
    let m: OciManifest = serde_json::from_str(json).unwrap();
    assert_eq!(m.schema_version, 0);
}

#[test]
fn parse_multiple_layers() {
    let json = r#"{
        "schemaVersion": 2,
        "config": {"mediaType": "t", "digest": "sha256:c", "size": 1},
        "layers": [
            {"mediaType": "l", "digest": "sha256:a", "size": 1},
            {"mediaType": "l", "digest": "sha256:b", "size": 2},
            {"mediaType": "l", "digest": "sha256:c", "size": 3}
        ]
    }"#;
    let m: OciManifest = serde_json::from_str(json).unwrap();
    assert_eq!(m.layers.len(), 3);
    assert_eq!(m.config.digest, "sha256:c");
}

#[test]
fn descriptor_rename_media_type() {
    let json = r#"{"mediaType": "application/vnd.oci.image.layer.v1.tar+gzip", "digest": "sha256:abc", "size": 42}"#;
    let d: OciDescriptor = serde_json::from_str(json).unwrap();
    assert_eq!(d.media_type, "application/vnd.oci.image.layer.v1.tar+gzip");
    assert_eq!(d.size, 42);
}

#[test]
fn descriptor_clone_and_eq() {
    let d = OciDescriptor { media_type: "t".into(), digest: "d".into(), size: 9 };
    assert_eq!(d, d.clone());
}

#[test]
fn client_is_send_sync() {
    fn check<T: Send + Sync>() {}
    check::<RegistryClient>();
}

#[tokio::test]
async fn fetch_manifest_unreachable_errors() {
    let c = RegistryClient::new("http://127.0.0.1:1", "img");
    let r = c.fetch_manifest("tag").await;
    assert!(r.is_err());
    assert!(matches!(r, Err(RegistryError::RetryExhausted(_, _))));
}

#[tokio::test]
async fn fetch_blob_unreachable_errors() {
    let c = RegistryClient::new("http://127.0.0.1:1", "img");
    let r = c.fetch_blob("sha256:abc").await;
    assert!(r.is_err());
}

#[tokio::test]
async fn get_with_retry_gives_up_after_five() {
    let c = RegistryClient::new("http://127.0.0.1:1", "img");
    let r = c.get_with_retry("http://127.0.0.1:1/x").await;
    assert!(r.is_err());
    match r {
        Err(RegistryError::RetryExhausted(attempts, _)) => assert_eq!(attempts, 5),
        _ => panic!("expected RetryExhausted(5, _)"),
    }
}

#[test]
fn manifest_not_found_display_format() {
    let e = RegistryError::ManifestNotFound("myimg".into(), "latest".into());
    let s = e.to_string();
    assert!(s.contains("myimg"));
    assert!(s.contains("latest"));
}

#[test]
fn retry_exhausted_has_attempt_count() {
    if let RegistryError::RetryExhausted(n, _) = RegistryError::RetryExhausted(5, "x".into()) {
        assert_eq!(n, 5);
    } else {
        panic!("not retry exhausted");
    }
}
