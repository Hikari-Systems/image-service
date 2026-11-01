use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use super::image::{ExpectedVariants, ImageBackend, ImageRecord, ScaledImage};

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
    /// Claim incomplete images for the background sweep.
    ///
    /// The whole design sits in one statement, and each part earns its place:
    ///
    /// * **`FOR UPDATE SKIP LOCKED`** is what makes this safe on a fleet. Two
    ///   nodes running it at the same instant step over each other's in-flight
    ///   rows instead of blocking on them, so neither waits and neither gets a
    ///   duplicate. Without `SKIP LOCKED` the second node blocks and then claims
    ///   the *same* rows once the first commits.
    /// * **The lease is written in the same statement as the selection.** Select
    ///   then update would leave a window where a second node reads the row before
    ///   the first marks it.
    /// * **The subquery is required.** `FOR UPDATE` cannot be applied to the
    ///   target of an `UPDATE` directly, and `LIMIT` needs a defined row order.
    /// * **`avoid_resize_until` is the lease.** A row is eligible when it has no
    ///   lease or the lease has expired, so a node that dies mid-transcode frees
    ///   its work automatically once the clock passes — no stuck rows, no operator.
    /// * **Incompleteness is per category.** `unnest` joins the caller's
    ///   (category, expected-count) pairs so a 2-variant `userIcon` is not judged
    ///   against a 3-variant `auctionPhoto`. Rows that are complete stop matching
    ///   and the sweep goes quiet, rather than re-leasing the same rows forever.
    /// * **`downloaded_s3_path IS NOT NULL`** because the transcode re-downloads
    ///   the source from it; a row without one has nothing to work from.
    async fn claim_for_transcode(
        &self,
        limit: i64,
        lease_seconds: i64,
        expected: &[ExpectedVariants],
        default_expected: i32,
    ) -> Result<Vec<ImageRecord>> {
        let cats: Vec<String> = expected.iter().map(|e| e.category.clone()).collect();
        let counts: Vec<i32> = expected.iter().map(|e| e.count).collect();

        let rows = sqlx::query_as::<_, ImageRow>(
            r#"
            UPDATE image SET avoid_resize_until = now() + make_interval(secs => $1)
            WHERE id IN (
                SELECT i.id
                FROM image i
                LEFT JOIN unnest($2::text[], $3::int[]) AS e(cat, n)
                       ON lower(coalesce(i.category, '')) = e.cat
                WHERE i.downloaded_s3_path IS NOT NULL
                  AND (i.avoid_resize_until IS NULL OR i.avoid_resize_until <= now())
                  AND (
                        i.original_s3_path IS NULL
                     OR jsonb_array_length(coalesce(i.resized_files, '[]'::jsonb))
                        < coalesce(e.n, $4)
                  )
                ORDER BY i.created_at NULLS FIRST
                LIMIT $5
                FOR UPDATE OF i SKIP LOCKED
            )
            RETURNING id, category, source_url, downloaded_s3_path, original_s3_path,
                      resized_files, avoid_resize_until, created_at
            "#,
        )
        .bind(lease_seconds as f64)
        .bind(&cats)
        .bind(&counts)
        .bind(default_expected)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("DB error claiming images for transcode")?;

        rows.into_iter().map(row_to_record).collect()
    }

    async fn set_resize_lease(&self, id: Uuid, until: Option<DateTime<Utc>>) -> Result<()> {
        sqlx::query("UPDATE image SET avoid_resize_until = $2 WHERE id = $1")
            .bind(id)
            .bind(until)
            .execute(&self.pool)
            .await
            .context("DB error setting resize lease")?;
        Ok(())
    }

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
