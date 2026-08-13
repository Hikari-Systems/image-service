use anyhow::{Context, Result};
use hs_utils::config::{
    apply_env_overrides, deep_merge, deser_i64_or_str, deser_opt_bool_or_str, deser_opt_i32_or_str,
    deser_u16_or_str, deser_u32_or_str, prepare_config,
};
pub use hs_utils::db::DbConfig;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

// ── Structs mirror the original TypeScript config.json exactly ──────────────

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_port", deserialize_with = "deser_u16_or_str")]
    pub port: u16,
}

fn default_port() -> u16 { 3000 }

#[derive(Debug, Deserialize, Clone)]
pub struct LogConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_log_level() -> String { "info".to_string() }

#[derive(Debug, Deserialize, Clone)]
pub struct ImageMetadataConfig {
    #[serde(rename = "parentPath", default = "default_metadata_path")]
    pub parent_path: String,
    #[serde(default = "default_storage")]
    pub storage: String,
}

fn default_metadata_path() -> String { "/metadata".to_string() }
fn default_storage() -> String { "file".to_string() }

#[derive(Debug, Deserialize, Clone)]
pub struct ImageMagickConfig {
    #[serde(default = "default_magick_bin")]
    pub bin: String,
}

fn default_magick_bin() -> String { "/usr/local/bin/magick".to_string() }

#[derive(Debug, Deserialize, Clone, Default)]
pub struct S3Config {
    #[serde(rename = "bucketName", default)]
    pub bucket_name: String,
    #[serde(rename = "accessKeyId", default)]
    pub access_key_id: String,
    #[serde(rename = "secretAccessKey", default)]
    pub secret_access_key: String,
    #[serde(default = "default_region")]
    pub region: String,
    /// Override the S3 endpoint — set this to point at MinIO or another S3-compatible
    /// server for local testing. Empty means the real AWS endpoint for the region.
    #[serde(rename = "endpointUrl", default)]
    pub endpoint_url: String,
    /// Path-style addressing (`host/bucket/key` rather than `bucket.host/key`).
    /// Defaults to on whenever `endpointUrl` is set, since S3-compatible servers are
    /// rarely reachable under per-bucket subdomains.
    #[serde(
        rename = "forcePathStyle",
        default,
        deserialize_with = "deser_opt_bool_or_str"
    )]
    pub force_path_style: Option<bool>,
}

impl S3Config {
    pub fn force_path_style(&self) -> bool {
        self.force_path_style
            .unwrap_or(!self.endpoint_url.trim().is_empty())
    }
}

fn default_region() -> String { "us-east-1".to_string() }

#[derive(Debug, Deserialize, Clone, Default)]
pub struct CloudfrontConfig {
    #[serde(default)]
    pub url: String,
    #[serde(
        rename = "expirySeconds",
        default = "default_expiry",
        deserialize_with = "deser_i64_or_str"
    )]
    pub expiry_seconds: i64,
    #[serde(rename = "keypairId", default)]
    pub keypair_id: String,
    #[serde(rename = "privateKey", default)]
    pub private_key: String,
    #[serde(rename = "privateKeyFile", default)]
    pub private_key_file: String,
}

fn default_expiry() -> i64 { 10100 }


#[derive(Debug, Deserialize, Clone, Default)]
pub struct SizeConfig {
    #[serde(deserialize_with = "deser_opt_i32_or_str", default)]
    pub width: Option<i32>,
    #[serde(deserialize_with = "deser_opt_i32_or_str", default)]
    pub height: Option<i32>,
    #[serde(rename = "mimeType")]
    pub mime_type: Option<String>,
    pub extension: Option<String>,
    #[serde(rename = "extraOpts", default)]
    pub extra_opts: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ResizeConfig {
    #[serde(default = "default_processing")]
    pub processing: String,
    #[serde(rename = "sizeKeys", default)]
    pub size_keys: String,
    #[serde(
        rename = "memoryLimit",
        default = "default_memory_limit",
        deserialize_with = "deser_u32_or_str"
    )]
    pub memory_limit: u32,
    #[serde(
        rename = "mapLimit",
        default = "default_map_limit",
        deserialize_with = "deser_u32_or_str"
    )]
    pub map_limit: u32,
    #[serde(default)]
    pub original: SizeConfig,
    /// Named size configs (small, medium, large, etc.) captured via flatten.
    #[serde(flatten)]
    pub sizes: HashMap<String, SizeConfig>,
    #[serde(rename = "scalingSets", default)]
    pub scaling_sets: HashMap<String, String>,
}

fn default_processing() -> String { "deferred".to_string() }
fn default_memory_limit() -> u32 { 32 }
fn default_map_limit() -> u32 { 32 }

impl ResizeConfig {
    pub fn size_keys_for_category(&self, category: &str) -> Vec<String> {
        let keys_str = if category.is_empty() {
            self.size_keys.clone()
        } else {
            self.scaling_sets
                .get(category)
                .cloned()
                .unwrap_or_else(|| self.size_keys.clone())
        };
        keys_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    pub fn get_size(&self, key: &str) -> Option<&SizeConfig> {
        self.sizes.get(key)
    }
}

#[derive(Debug, Deserialize, Clone, Default)]
#[allow(dead_code)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(rename = "imageMetadata", default)]
    pub image_metadata: ImageMetadataConfig,
    #[serde(default)]
    pub imagemagick: ImageMagickConfig,
    #[serde(rename = "uploadDir", default)]
    pub upload_dir: String,
    #[serde(default)]
    pub s3: S3Config,
    #[serde(default)]
    pub cloudfront: CloudfrontConfig,
    #[serde(default)]
    pub db: DbConfig,
    #[serde(default)]
    pub resize: ResizeConfig,
}

impl Default for ServerConfig {
    fn default() -> Self { Self { port: default_port() } }
}
impl Default for LogConfig {
    fn default() -> Self { Self { level: default_log_level() } }
}
impl Default for ImageMetadataConfig {
    fn default() -> Self {
        Self { parent_path: default_metadata_path(), storage: default_storage() }
    }
}
impl Default for ImageMagickConfig {
    fn default() -> Self { Self { bin: default_magick_bin() } }
}

impl AppConfig {
    /// Load configuration in priority order (lowest → highest):
    ///
    /// 1. `config.json` in the working directory
    /// 2. `/sandbox/config.json` — deep-merged on top; silently ignored if absent
    /// 3. Env vars with `__` separator, e.g. `s3__bucketName=my-bucket`
    pub fn load() -> Result<Self> {
        let mut root: Value = match std::fs::read_to_string("config.json") {
            Ok(s) => serde_json::from_str(&s).context("Failed to parse config.json")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Value::Object(Default::default())
            }
            Err(e) => return Err(e).context("Failed to read config.json"),
        };

        match std::fs::read_to_string("/sandbox/config.json") {
            Ok(s) => {
                let overlay: Value = serde_json::from_str(&s)
                    .context("Failed to parse /sandbox/config.json")?;
                deep_merge(&mut root, overlay);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Failed to read /sandbox/config.json"),
        }

        prepare_config(&mut root);
        apply_env_overrides(&mut root);
        serde_json::from_value(root).context("Failed to deserialise config")
    }
}
