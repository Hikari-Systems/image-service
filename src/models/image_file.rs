use anyhow::{Context, Result};
use async_trait::async_trait;
use std::path::PathBuf;
use tokio::fs;
use tracing::{error, warn};
use uuid::Uuid;

use super::image::{ImageBackend, ImageRecord};

pub struct FileBackend {
    parent_path: String,
}

impl FileBackend {
    pub fn new(parent_path: String) -> Self {
        Self { parent_path }
    }

    fn path_for(&self, id: &str) -> PathBuf {
        PathBuf::from(&self.parent_path).join(format!("{}.json", id))
    }
}

#[async_trait]
impl ImageBackend for FileBackend {
    async fn get(&self, id: &str) -> Result<Option<ImageRecord>> {
        let path = self.path_for(id);
        match fs::read_to_string(&path).await {
            Ok(json) => {
                let record: ImageRecord = serde_json::from_str(&json)
                    .with_context(|| format!("Failed to parse image JSON for id={}", id))?;
                Ok(Some(record))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => {
                error!("Error loading image details for id={}: {}", id, e);
                Err(e).with_context(|| format!("Failed to read image file for id={}", id))
            }
        }
    }

    async fn upsert(&self, image: ImageRecord) -> Result<ImageRecord> {
        let record = ImageRecord {
            id: Some(image.id.unwrap_or_else(Uuid::new_v4)),
            ..image
        };
        let id_str = record.id.unwrap().to_string();
        let path = self.path_for(&id_str);
        let json = serde_json::to_string(&record)
            .with_context(|| format!("Failed to serialise image id={}", id_str))?;
        fs::write(&path, json)
            .await
            .with_context(|| format!("Failed to write image file for id={}", id_str))
            .map_err(|e| {
                warn!("Error saving image id={}: {}", id_str, e);
                e
            })?;
        Ok(record)
    }
}
