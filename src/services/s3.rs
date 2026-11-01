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
        let mut builder = aws_sdk_s3::Config::builder()
            .credentials_provider(credentials)
            .region(Region::new(cfg.region.clone()))
            .force_path_style(cfg.force_path_style())
            .behavior_version_latest();

        // Point at MinIO (or any S3-compatible server) when configured; otherwise the
        // SDK resolves the real AWS endpoint for the region.
        let endpoint = cfg.endpoint_url.trim();
        if !endpoint.is_empty() {
            info!("S3 endpoint override: {}", endpoint);
            builder = builder.endpoint_url(endpoint);
        }

        let client = Client::from_conf(builder.build());
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
