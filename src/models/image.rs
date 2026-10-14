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

/// One category's expected variant count, for the sweep's incompleteness test.
/// Built from `resize.scalingSets` so the query can ask "does this row have all
/// the sizes *its own* category configures?" rather than one global number.
pub struct ExpectedVariants {
    /// Lower-cased category name — categories are lower-cased before the scaling
    /// set is looked up, so the comparison must be too.
    pub category: String,
    pub count: i32,
}

#[async_trait]
pub trait ImageBackend: Send + Sync {
    async fn get(&self, id: &str) -> Result<Option<ImageRecord>>;
    async fn upsert(&self, image: ImageRecord) -> Result<ImageRecord>;

    /// Atomically take ownership of up to `limit` images whose variants are
    /// incomplete, holding each for `lease_seconds`.
    ///
    /// The contract that matters is **exclusivity**: two nodes running this
    /// concurrently must never receive the same image. Callers rely on that to
    /// avoid paying for the same transcode twice and to avoid two writers racing
    /// on one row's `resized_files`.
    ///
    /// Returns an empty vec on backends that cannot guarantee it — a claim that
    /// is merely *probably* exclusive is worse than none, because it looks like
    /// it works.
    async fn claim_for_transcode(
        &self,
        limit: i64,
        lease_seconds: i64,
        expected: &[ExpectedVariants],
        default_expected: i32,
    ) -> Result<Vec<ImageRecord>> {
        let _ = (limit, lease_seconds, expected, default_expected);
        Ok(Vec::new())
    }

    /// Set (or clear) an image's resize lease without touching anything else.
    ///
    /// Deliberately not `upsert`: the sweep must not write back a whole record it
    /// read seconds ago, because the transcode it just ran has already updated
    /// that row and a full write would stamp the stale copy over it.
    async fn set_resize_lease(&self, id: Uuid, until: Option<DateTime<Utc>>) -> Result<()> {
        let _ = (id, until);
        Ok(())
    }
}
