use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

// ── Structs mirror the original TypeScript config.json exactly ──────────────

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
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
}

fn default_region() -> String { "us-east-1".to_string() }

#[derive(Debug, Deserialize, Clone, Default)]
pub struct CloudfrontConfig {
    #[serde(default)]
    pub url: String,
    #[serde(rename = "expirySeconds", default = "default_expiry")]
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
pub struct DbSslConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub verify: bool,
    #[serde(rename = "caCertFile", default)]
    pub ca_cert_file: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
#[allow(dead_code)]
pub struct DbConfig {
    #[serde(default = "default_db_host")]
    pub host: String,
    #[serde(default = "default_db_port")]
    pub port: u16,
    #[serde(default)]
    pub database: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub ssl: DbSslConfig,
    #[serde(default)]
    pub minpool: u32,
    #[serde(default = "default_maxpool")]
    pub maxpool: u32,
    #[serde(default)]
    pub debug: bool,
}

fn default_db_host() -> String { "localhost".to_string() }
fn default_db_port() -> u16 { 5432 }
fn default_maxpool() -> u32 { 10 }

#[derive(Debug, Deserialize, Clone, Default)]
pub struct SizeConfig {
    pub width: Option<i32>,
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
    #[serde(rename = "memoryLimit", default = "default_memory_limit")]
    pub memory_limit: u32,
    #[serde(rename = "mapLimit", default = "default_map_limit")]
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
    fn default() -> Self { Self { parent_path: default_metadata_path(), storage: default_storage() } }
}
impl Default for ImageMagickConfig {
    fn default() -> Self { Self { bin: default_magick_bin() } }
}

impl AppConfig {
    /// Load configuration in priority order (lowest → highest):
    ///
    /// 1. `config.json` in the working directory (baked into the image, default values)
    /// 2. The file pointed to by the `CONFIG_FILE` environment variable, deep-merged
    ///    on top — used to inject secrets/environment-specific values via a mounted volume
    /// 3. Individual environment variables using `__` as the path separator, e.g.
    ///    `imageMetadata__storage=db` or `s3__bucketName=my-bucket`
    pub fn load() -> Result<Self> {
        // 1. Base config.
        let mut root: Value = match std::fs::read_to_string("config.json") {
            Ok(s) => serde_json::from_str(&s).context("Failed to parse config.json")?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Value::Object(Default::default()),
            Err(e) => return Err(e).context("Failed to read config.json"),
        };

        // 2. Optional sandbox override — deep-merged on top of the base.
        //    /sandbox/config.json is mounted as a volume in production and contains
        //    environment-specific values (secrets, bucket names, etc.).
        //    Silently ignored if the file doesn't exist (local dev without a mount).
        match std::fs::read_to_string("/sandbox/config.json") {
            Ok(s) => {
                let overlay: Value = serde_json::from_str(&s)
                    .context("Failed to parse /sandbox/config.json")?;
                deep_merge(&mut root, overlay);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).context("Failed to read /sandbox/config.json"),
        }

        // 3. Per-key env var overrides.
        //    Must match config.json key names exactly (case-sensitive).
        //    e.g. `imageMetadata__parentPath=foo` → imageMetadata.parentPath = "foo"
        for (raw_key, val) in std::env::vars() {
            let parts: Vec<String> = raw_key.split("__").map(|s| s.to_string()).collect();
            if parts.len() < 2 {
                continue;
            }
            set_nested(&mut root, &parts, val);
        }

        // 4. Deserialize — serde honours the #[serde(rename = "...")] annotations.
        serde_json::from_value(root).context("Failed to deserialise config")
    }
}

/// Recursively merge `overlay` into `base`. Object keys are merged; all other
/// types (strings, numbers, arrays) are replaced by the overlay value.
fn deep_merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base_map), Value::Object(overlay_map)) => {
            for (k, v) in overlay_map {
                deep_merge(base_map.entry(k).or_insert(Value::Null), v);
            }
        }
        (base, overlay) => *base = overlay,
    }
}

/// Walk `node` using exact-match `path` segments and set the leaf to `val`.
/// The value is coerced to match the type already present at that path
/// (bool, integer, float, or string).
fn set_nested(node: &mut Value, path: &[String], val: String) {
    let Value::Object(map) = node else { return };

    let key = path[0].clone();

    if path.len() == 1 {
        // Leaf — coerce to the type of the existing value if possible.
        let existing = map.get(&key);
        let coerced = coerce_value(&val, existing);
        map.insert(key, coerced);
    } else {
        // Intermediate node — recurse, inserting an empty object if missing.
        let child = map
            .entry(key)
            .or_insert_with(|| Value::Object(Default::default()));
        set_nested(child, &path[1..], val);
    }
}

/// Attempt to parse `s` into the same JSON type as `hint`, falling back to a string.
fn coerce_value(s: &str, hint: Option<&Value>) -> Value {
    match hint {
        Some(Value::Bool(_)) => match s.to_lowercase().as_str() {
            "true" | "1" | "yes" => Value::Bool(true),
            _ => Value::Bool(false),
        },
        Some(Value::Number(n)) => {
            if n.is_f64() {
                if let Ok(f) = s.parse::<f64>() {
                    return serde_json::json!(f);
                }
            } else if let Ok(i) = s.parse::<i64>() {
                return serde_json::json!(i);
            }
            Value::String(s.to_string())
        }
        _ => Value::String(s.to_string()),
    }
}
