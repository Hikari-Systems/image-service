use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScaledImage {
    pub size: String,
    #[serde(rename = "s3Path")]
    pub s3_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageRecord {
    pub id: Option<Uuid>,
    pub category: Option<String>,
    #[serde(rename = "sourceUrl", skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(rename = "downloadedS3Path", skip_serializing_if = "Option::is_none")]
    pub downloaded_s3_path: Option<String>,
    #[serde(rename = "originalS3Path", skip_serializing_if = "Option::is_none")]
    pub original_s3_path: Option<String>,
    #[serde(rename = "resizedFiles", skip_serializing_if = "Option::is_none")]
    pub resized_files: Option<Vec<ScaledImage>>,
    #[serde(rename = "avoidResizeUntil", skip_serializing_if = "Option::is_none")]
    pub avoid_resize_until: Option<DateTime<Utc>>,
    #[serde(rename = "createdAt", skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
}

/// The image descriptor returned to callers, which augments the stored record
/// with a pre-signed download URL for the original/downloaded file.
#[derive(Debug, Clone, Serialize)]
pub struct ImageDescriptor {
    #[serde(flatten)]
    pub record: ImageRecord,
    #[serde(rename = "originalFileUrl")]
    pub original_file_url: String,
}

#[async_trait]
pub trait ImageBackend: Send + Sync {
    async fn get(&self, id: &str) -> Result<Option<ImageRecord>>;
    async fn upsert(&self, image: ImageRecord) -> Result<ImageRecord>;
}
