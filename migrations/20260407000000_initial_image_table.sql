CREATE TABLE IF NOT EXISTS image (
    id                 UUID PRIMARY KEY NOT NULL,
    category           VARCHAR(255),
    source_url         TEXT,
    downloaded_s3_path TEXT,
    original_s3_path   TEXT,
    resized_files      JSONB,
    avoid_resize_until TIMESTAMPTZ,
    created_at         TIMESTAMPTZ,
    updated_at         TIMESTAMPTZ
);
