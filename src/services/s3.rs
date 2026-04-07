use anyhow::{Context, Result};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::Region;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client;
use std::path::Path;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, info};

use crate::config::S3Config;

pub struct S3Service {
    client: Client,
    bucket: String,
}

impl S3Service {
    pub fn new(cfg: &S3Config) -> Self {
        let credentials = Credentials::new(
            &cfg.access_key_id,
            &cfg.secret_access_key,
            None,
            None,
            "config",
        );
        let sdk_config = aws_sdk_s3::Config::builder()
            .credentials_provider(credentials)
            .region(Region::new(cfg.region.clone()))
            .behavior_version_latest()
            .build();
        let client = Client::from_conf(sdk_config);
        Self {
            client,
            bucket: cfg.bucket_name.clone(),
        }
    }

    pub async fn save(&self, from: &Path, to: &str, mime_type: &str) -> Result<()> {
        // Read into memory first — ByteStream::from_path with behavior_version_latest()
        // can deadlock when the SDK tries to compute a streaming checksum on a file body.
        let bytes = tokio::fs::read(from)
            .await
            .with_context(|| format!("Failed to read {:?} for S3 upload", from))?;
        info!("S3 upload: {:?} ({} bytes) → {}", from, bytes.len(), to);
        let body = ByteStream::from(bytes);
        debug!("S3 upload body ready: {} → {}", from.display(), to);
        timeout(
            Duration::from_secs(120),
            self.client
                .put_object()
                .bucket(&self.bucket)
                .key(to)
                .body(body)
                .content_type(mime_type)
                .send(),
        )
        .await
        .with_context(|| format!("S3 PutObject timed out after 120s for key={}", to))?
        .with_context(|| format!("S3 PutObject failed for key={}", to))?;
        Ok(())
    }
}
