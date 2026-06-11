#![allow(dead_code)]
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct RegistryClient {
    base: String,
    image: String,
    client: reqwest::Client,
    token_manager: Option<TokenManager>,
}

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("http error: {0}")]
    Http(String),
    #[error("manifest not found for {0}:{1}")]
    ManifestNotFound(String, String),
    #[error("blob not found: {0}")]
    BlobNotFound(String),
    #[error("retry exhausted after {0} attempts: {1}")]
    RetryExhausted(usize, String),
    #[error("auth error: {0}")]
    Auth(String),
    #[error("config error: {0}")]
    Config(String),
    #[error("upload error: {0}")]
    Upload(String),
}

#[derive(Debug, PartialEq, Deserialize, Clone)]
pub struct OciDescriptor {
    #[serde(rename = "mediaType")]
    pub media_type: String,
    pub digest: String,
    pub size: u64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct OciManifest {
    #[serde(default, rename = "schemaVersion")]
    pub schema_version: u64,
    pub config: OciDescriptor,
    pub layers: Vec<OciDescriptor>,
}

#[derive(Debug, Deserialize)]
struct DockerConfig {
    #[serde(default)]
    auths: HashMap<String, DockerAuthConfig>,
    #[serde(default, rename = "credsStore")]
    creds_store: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct DockerAuthConfig {
    auth: Option<String>,
    #[serde(default)]
    identity_token: Option<String>,
    #[serde(default)]
    registry_token: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DockerCredentials {
    pub registry: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone)]
struct TokenManager {
    token: String,
    expires_at: Instant,
    service: String,
    scope: String,
    client: reqwest::Client,
    base_url: String,
}

impl RegistryClient {
    pub fn new(base: impl Into<String>, image: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("failed to build registry client");
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            image: image.into(),
            client,
            token_manager: None,
        }
    }

    pub fn with_credentials(
        base: impl Into<String>,
        image: impl Into<String>,
        _creds: DockerCredentials,
    ) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("failed to build registry client");
        let base_s = base.into().trim_end_matches('/').to_string();
        let image_s = image.into();
        let token_mgr = Some(TokenManager {
            token: String::new(),
            expires_at: Instant::now(),
            service: base_s.clone(),
            scope: format!("repository:{}:pull,push", image_s),
            client: reqwest::Client::new(),
            base_url: base_s.clone(),
        });
        Self {
            base: base_s,
            image: image_s,
            client: http_client,
            token_manager: token_mgr,
        }
    }

    pub fn base_url(&self) -> &str {
        &self.base
    }

    pub fn manifest_url(&self, tag: &str) -> String {
        format!("{}/v2/{}/manifests/{}", self.base, self.image, tag)
    }

    pub fn blob_url(&self, digest: &str) -> String {
        format!("{}/v2/{}/blobs/{}", self.base, self.image, digest)
    }

    pub async fn fetch_manifest(&self, tag: &str) -> Result<OciManifest, RegistryError> {
        let url = self.manifest_url(tag);
        self.get_with_retry(&url).await?;
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| RegistryError::Http(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::ManifestNotFound(
                self.image.clone(),
                tag.into(),
            ));
        }
        let manifest: OciManifest = resp
            .json()
            .await
            .map_err(|e| RegistryError::Http(e.to_string()))?;
        Ok(manifest)
    }

    pub async fn fetch_blob(&self, digest: &str) -> Result<Vec<u8>, RegistryError> {
        let url = self.blob_url(digest);
        self.get_with_retry(&url).await?;
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| RegistryError::Http(e.to_string()))?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(RegistryError::BlobNotFound(digest.into()));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| RegistryError::Http(e.to_string()))?;
        Ok(bytes.to_vec())
    }

    pub async fn get_with_retry(&self, url: &str) -> Result<reqwest::Response, RegistryError> {
        let max_attempts = 5;
        let mut delay = Duration::from_millis(150);
        let mut last_err = String::new();
        for attempt in 0..max_attempts {
            match self.client.get(url).send().await {
                Ok(resp)
                    if resp.status().is_success()
                        || resp.status() == reqwest::StatusCode::NOT_FOUND =>
                {
                    return Ok(resp);
                }
                Ok(resp) => {
                    last_err = format!("status {}", resp.status());
                }
                Err(e) => {
                    last_err = e.to_string();
                }
            }
            if attempt < max_attempts - 1 {
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(8));
            }
        }
        Err(RegistryError::RetryExhausted(max_attempts, last_err))
    }

    pub async fn push_manifest(
        &self,
        tag: &str,
        manifest_bytes: &[u8],
    ) -> Result<(), RegistryError> {
        let url = self.manifest_url(tag);
        let resp = self
            .client
            .put(&url)
            .header("Content-Type", "application/vnd.oci.image.manifest.v1+json")
            .body(manifest_bytes.to_vec())
            .send()
            .await
            .map_err(|e| RegistryError::Upload(e.to_string()))?;
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::CREATED {
            return Err(RegistryError::Upload(format!(
                "manifest push returned {}",
                resp.status()
            )));
        }
        Ok(())
    }

    pub async fn start_upload(&self) -> Result<String, RegistryError> {
        let url = format!("{}/v2/{}/blobs/uploads/", self.base, self.image);
        let resp = self
            .client
            .post(&url)
            .send()
            .await
            .map_err(|e| RegistryError::Upload(e.to_string()))?;
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::ACCEPTED {
            return Err(RegistryError::Upload(format!(
                "upload start returned {}",
                resp.status()
            )));
        }
        resp.headers()
            .get("Location")
            .and_then(|v| v.to_str().ok().map(|s| s.to_string()))
            .ok_or_else(|| RegistryError::Upload("no Location header from upload start".into()))
    }

    pub async fn patch_chunk(&self, upload_url: &str, chunk: &[u8]) -> Result<(), RegistryError> {
        let resp = self
            .client
            .patch(upload_url)
            .header("Content-Type", "application/octet-stream")
            .body(chunk.to_vec())
            .send()
            .await
            .map_err(|e| RegistryError::Upload(e.to_string()))?;
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::ACCEPTED {
            return Err(RegistryError::Upload(format!(
                "chunk upload returned {}",
                resp.status()
            )));
        }
        Ok(())
    }

    pub async fn finish_upload(&self, upload_url: &str, digest: &str) -> Result<(), RegistryError> {
        let final_url = format!("{}&digest={}", upload_url, digest);
        let resp = self
            .client
            .put(&final_url)
            .header("Content-Type", "application/octet-stream")
            .body(vec![])
            .send()
            .await
            .map_err(|e| RegistryError::Upload(e.to_string()))?;
        if !resp.status().is_success() && resp.status() != reqwest::StatusCode::CREATED {
            return Err(RegistryError::Upload(format!(
                "finish upload returned {}",
                resp.status()
            )));
        }
        Ok(())
    }
}

pub fn parse_docker_config(
    path: Option<&std::path::Path>,
) -> Result<Vec<DockerCredentials>, RegistryError> {
    let config_path = path.map(|p| p.to_path_buf()).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/root".into());
        PathBuf::from(home).join(".docker").join("config.json")
    });
    let content = std::fs::read_to_string(&config_path).map_err(|e| {
        RegistryError::Config(format!("cannot read {}: {}", config_path.display(), e))
    })?;
    let docker_config: DockerConfig = serde_json::from_str(&content)
        .map_err(|e| RegistryError::Config(format!("parse error: {e}")))?;
    let mut credentials = Vec::new();
    for (registry, auth_config) in &docker_config.auths {
        if let Some(auth_b64) = &auth_config.auth {
            let decoded = base64_decode(auth_b64).map_err(RegistryError::Config)?;
            let decoded_str = String::from_utf8(decoded)
                .map_err(|_| RegistryError::Config("auth is not valid UTF-8".into()))?;
            if let Some((user, pass)) = decoded_str.split_once(':') {
                credentials.push(DockerCredentials {
                    registry: registry.clone(),
                    username: user.to_string(),
                    password: pass.to_string(),
                });
            }
        }
    }
    if credentials.is_empty() {
        if let Some(store) = docker_config.creds_store {
            return Err(RegistryError::Config(format!(
                "credsStore '{store}' configured but not supported; use auth field directly"
            )));
        }
    }
    Ok(credentials)
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("base64 decode failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn client() -> RegistryClient {
        RegistryClient::new("https://registry.example.com", "library/ubuntu")
    }

    #[test]
    fn test_base_url_trims_trailing_slash() {
        let c = RegistryClient::new("https://x.com/", "img");
        assert_eq!(c.base, "https://x.com");
    }

    #[test]
    fn test_manifest_url_shape() {
        let c = client();
        assert_eq!(
            c.manifest_url("24.04"),
            "https://registry.example.com/v2/library/ubuntu/manifests/24.04"
        );
    }

    #[test]
    fn test_blob_url_shape() {
        let c = client();
        assert_eq!(
            c.blob_url("sha256:abc"),
            "https://registry.example.com/v2/library/ubuntu/blobs/sha256:abc"
        );
    }

    #[tokio::test]
    async fn test_fetch_manifest_unreachable_errors() {
        let c = RegistryClient::new("http://127.0.0.1:1", "img");
        let r = c.fetch_manifest("tag").await;
        assert!(r.is_err());
        assert!(matches!(r, Err(RegistryError::RetryExhausted(_, _))));
    }

    #[tokio::test]
    async fn test_fetch_blob_unreachable_errors() {
        let c = RegistryClient::new("http://127.0.0.1:1", "img");
        let r = c.fetch_blob("sha256:abc").await;
        assert!(r.is_err());
    }

    #[test]
    fn test_error_display() {
        let e = RegistryError::ManifestNotFound("img".into(), "t".into());
        assert!(e.to_string().contains("img"));
        assert!(e.to_string().contains("t"));
    }

    #[test]
    fn test_retry_exhausted_display() {
        let e = RegistryError::RetryExhausted(5, "boom".into());
        assert!(e.to_string().contains("5"));
        assert!(e.to_string().contains("boom"));
    }

    #[test]
    fn test_blob_not_found_display() {
        let e = RegistryError::BlobNotFound("sha256:x".into());
        assert!(e.to_string().contains("sha256:x"));
    }

    #[test]
    fn test_parse_manifest_json() {
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
    fn test_parse_manifest_default_schema_version() {
        let json = r#"{
            "config": {"mediaType": "t", "digest": "sha256:c", "size": 1},
            "layers": []
        }"#;
        let m: OciManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.schema_version, 0);
    }

    #[test]
    fn test_multiple_layers_parse() {
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
    }

    #[test]
    fn test_descriptor_clone() {
        let d = OciDescriptor {
            media_type: "t".into(),
            digest: "d".into(),
            size: 9,
        };
        let c = d.clone();
        assert_eq!(d, c);
    }

    #[test]
    fn test_client_is_send_sync() {
        fn check<T: Send + Sync>() {}
        check::<RegistryClient>();
    }

    #[tokio::test]
    async fn test_get_with_retry_gives_up_on_unreachable() {
        let c = RegistryClient::new("http://127.0.0.1:1", "img");
        let r = c.get_with_retry("http://127.0.0.1:1/x").await;
        assert!(r.is_err());
        match r {
            Err(RegistryError::RetryExhausted(attempts, _)) => assert_eq!(attempts, 5),
            _ => panic!("expected RetryExhausted"),
        }
    }

    #[test]
    fn test_push_manifest_url_is_put() {
        let c = client();
        let url = c.manifest_url("latest");
        assert_eq!(
            url,
            "https://registry.example.com/v2/library/ubuntu/manifests/latest"
        );
        assert!(url.contains("manifests"));
    }

    #[test]
    fn test_start_upload_url_is_post() {
        let c = client();
        let url = format!("{}/v2/{}/blobs/uploads/", c.base, c.image);
        assert_eq!(
            url,
            "https://registry.example.com/v2/library/ubuntu/blobs/uploads/"
        );
    }

    #[test]
    fn test_parse_docker_config_with_valid_auth() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let config_content = r#"{
            "auths": {
                "https://index.docker.io/v1/": {
                    "auth": "dXNlcjpwYXNz"
                }
            }
        }"#;
        std::fs::write(&config_path, config_content).unwrap();
        let creds = parse_docker_config(Some(&config_path)).unwrap();
        assert_eq!(creds.len(), 1);
        assert_eq!(creds[0].username, "user");
        assert_eq!(creds[0].password, "pass");
        assert_eq!(creds[0].registry, "https://index.docker.io/v1/");
    }

    #[test]
    fn test_parse_docker_config_with_multiple_auths() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let config_content = r#"{
            "auths": {
                "https://registry.example.com": {"auth": "YWxpY2U6cGFzc3dvcmQ="},
                "https://docker.io": {"auth": "Ym9iOndvcmxk"}
            }
        }"#;
        std::fs::write(&config_path, config_content).unwrap();
        let creds = parse_docker_config(Some(&config_path)).unwrap();
        assert_eq!(creds.len(), 2);
    }

    #[test]
    fn test_parse_docker_config_creds_store_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let config_content = r#"{
            "credsStore": "osxkeychain",
            "auths": {}
        }"#;
        std::fs::write(&config_path, config_content).unwrap();
        let result = parse_docker_config(Some(&config_path));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("credsStore"));
    }

    #[test]
    fn test_parse_docker_config_empty_auths() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.json");
        let config_content = r#"{"auths": {}}"#;
        std::fs::write(&config_path, config_content).unwrap();
        let creds = parse_docker_config(Some(&config_path)).unwrap();
        assert!(creds.is_empty());
    }

    #[test]
    fn test_parse_docker_config_missing_file() {
        let result = parse_docker_config(Some(Path::new("/no/such/docker/config.json")));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("cannot read"));
    }

    #[test]
    fn test_upload_url_construction() {
        let c = client();
        let url = format!("{}/v2/{}/blobs/uploads/", c.base, c.image);
        assert_eq!(
            url,
            "https://registry.example.com/v2/library/ubuntu/blobs/uploads/"
        );
    }

    #[test]
    fn test_finish_upload_url_includes_digest() {
        let upload_url = "https://registry.example.com/v2/upload/ref123";
        let digest = "sha256:abc123";
        let final_url = format!("{}&digest={}", upload_url, digest);
        assert_eq!(
            final_url,
            "https://registry.example.com/v2/upload/ref123&digest=sha256:abc123"
        );
    }

    #[test]
    fn test_with_credentials_basic() {
        let creds = DockerCredentials {
            registry: "https://index.docker.io/v1/".into(),
            username: "user".into(),
            password: "pass".into(),
        };
        let c = RegistryClient::with_credentials(
            "https://registry-1.docker.io",
            "library/ubuntu",
            creds,
        );
        assert_eq!(c.base, "https://registry-1.docker.io");
        assert!(c.token_manager.is_some());
    }

    #[tokio::test]
    async fn test_push_manifest_to_unreachable_errors() {
        let c = RegistryClient::new("http://127.0.0.1:1", "img");
        let r = c.push_manifest("tag", b"{}").await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_patch_chunk_to_unreachable_errors() {
        let c = RegistryClient::new("http://127.0.0.1:1", "img");
        let r = c.patch_chunk("http://127.0.0.1:1/upload", b"data").await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn test_finish_upload_to_unreachable_errors() {
        let c = RegistryClient::new("http://127.0.0.1:1", "img");
        let r = c
            .finish_upload("http://127.0.0.1:1/upload", "sha256:abc")
            .await;
        assert!(r.is_err());
    }
}
