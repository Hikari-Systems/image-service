use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::image::{ImageBackend, ImageRecord, ScaledImage};

pub struct DbBackend {
    pool: PgPool,
}

impl DbBackend {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Raw row from PostgreSQL. Uses `FromRow` derive for sqlx.
#[derive(FromRow)]
struct ImageRow {
    id: Uuid,
    category: Option<String>,
    source_url: Option<String>,
    downloaded_s3_path: Option<String>,
    original_s3_path: Option<String>,
    resized_files: Option<Value>,
    avoid_resize_until: Option<DateTime<Utc>>,
    created_at: Option<DateTime<Utc>>,
}

fn row_to_record(row: ImageRow) -> Result<ImageRecord> {
    let resized_files: Option<Vec<ScaledImage>> = match row.resized_files {
        Some(v) => Some(
            serde_json::from_value(v).context("Failed to deserialise resized_files from db")?,
        ),
        None => None,
    };
    Ok(ImageRecord {
        id: Some(row.id),
        category: row.category,
        source_url: row.source_url,
        downloaded_s3_path: row.downloaded_s3_path,
        original_s3_path: row.original_s3_path,
        resized_files,
        avoid_resize_until: row.avoid_resize_until,
        created_at: row.created_at,
    })
}

#[async_trait]
impl ImageBackend for DbBackend {
    async fn get(&self, id: &str) -> Result<Option<ImageRecord>> {
        let uuid: Uuid = id.parse().context("Invalid UUID format")?;

        let row = sqlx::query_as::<_, ImageRow>(
            r#"
            SELECT id, category, source_url, downloaded_s3_path, original_s3_path,
                   resized_files, avoid_resize_until, created_at
            FROM image
            WHERE id = $1
            "#,
        )
        .bind(uuid)
        .fetch_optional(&self.pool)
        .await
        .context("DB error fetching image")?;

        match row {
            Some(r) => Ok(Some(row_to_record(r)?)),
            None => Ok(None),
        }
    }

    async fn upsert(&self, image: ImageRecord) -> Result<ImageRecord> {
        let id = image.id.unwrap_or_else(Uuid::new_v4);
        let resized_files_json: Option<Value> = match &image.resized_files {
            Some(v) => Some(serde_json::to_value(v).context("Failed to serialise resized_files")?),
            None => None,
        };

        let row = sqlx::query_as::<_, ImageRow>(
            r#"
            INSERT INTO image (
                id, category, source_url, downloaded_s3_path, original_s3_path,
                resized_files, avoid_resize_until, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, now(), now())
            ON CONFLICT (id) DO UPDATE SET
                category           = EXCLUDED.category,
                source_url         = EXCLUDED.source_url,
                downloaded_s3_path = EXCLUDED.downloaded_s3_path,
                original_s3_path   = EXCLUDED.original_s3_path,
                resized_files      = EXCLUDED.resized_files,
                avoid_resize_until = EXCLUDED.avoid_resize_until,
                updated_at         = now()
            RETURNING id, category, source_url, downloaded_s3_path, original_s3_path,
                      resized_files, avoid_resize_until, created_at
            "#,
        )
        .bind(id)
        .bind(&image.category)
        .bind(&image.source_url)
        .bind(&image.downloaded_s3_path)
        .bind(&image.original_s3_path)
        .bind(resized_files_json)
        .bind(image.avoid_resize_until)
        .fetch_one(&self.pool)
        .await
        .context("DB error upserting image")?;

        row_to_record(row)
    }
}
