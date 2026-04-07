# image-service-rs

A Rust/actix-web microservice for image upload, transcoding, and delivery via AWS S3 and CloudFront. Drop-in replacement for the original TypeScript/Express image-service; all API endpoints, JSON config keys, and response shapes are identical.

---

## Features

- Multipart upload with automatic transcoding to multiple size variants
- ImageMagick-based resize and format conversion
- S3 storage for all image assets (original, processed original, size variants)
- CloudFront signed URL generation for time-limited delivery
- PostgreSQL or filesystem metadata backends (runtime-switchable)
- SVG sanitisation via `ammonia`
- Config layering: baked defaults → `/sandbox/config.json` → env vars

---

## API Endpoints

### `GET /healthcheck`
Returns `200 OK` with body `OK`. Used by Docker healthchecks and load balancers.

---

### `POST /api/image/:category`
Upload a new image. Accepts a `multipart/form-data` body with a field named `image`.

**Path params**
| Param | Description |
|---|---|
| `category` | Image category, e.g. `default`, `avatar`. Determines which `scalingSets` entry is used. |

**Query params**
| Param | Type | Description |
|---|---|---|
| `forceImmediateResize` | bool | If `true`, transcode synchronously before responding. Otherwise follows `resize.processing` config. |

**Response `201 Created`**
```json
{
  "id": "6a2ecdd6-63ee-47df-a886-0ba95e5cdeb4",
  "category": "default",
  "downloadedS3Path": "default-6a2ecdd6-63ee-47df-a886-0ba95e5cdeb4.jpg",
  "originalS3Path": "default-6a2ecdd6-63ee-47df-a886-0ba95e5cdeb4-original.png",
  "resizedFiles": [
    { "size": "small",  "s3Path": "default-6a2ecdd6-...-small.jpg" },
    { "size": "medium", "s3Path": "default-6a2ecdd6-...-medium.jpg" },
    { "size": "large",  "s3Path": "default-6a2ecdd6-...-large.png" }
  ],
  "createdAt": "2026-04-07T16:43:55.149Z"
}
```

---

### `GET /api/image/:id`
Retrieve image metadata with a pre-signed CloudFront URL for the original/downloaded file.

**Response `200 OK`**
```json
{
  "id": "6a2ecdd6-63ee-47df-a886-0ba95e5cdeb4",
  "category": "default",
  "downloadedS3Path": "...",
  "originalFileUrl": "https://cdn.example.com/default-6a2ecdd6-...?Expires=...&Signature=..."
}
```

Returns `404` if the image is not found.

---

### `GET /api/image/r/:id/:size`
Redirect (HTTP 302) to a signed CloudFront URL for the requested size variant. Falls back through: exact size → original → downloaded → sourceUrl.

**Path params**
| Param | Description |
|---|---|
| `id` | Image UUID |
| `size` | Size key, e.g. `small`, `medium`, `large` |

---

### `GET /api/image/s/:id/:size`
Same as the redirect endpoint but returns JSON instead of redirecting.

**Response `200 OK`**
```json
{ "url": "https://cdn.example.com/default-6a2ecdd6-...-small.jpg?Expires=...&Signature=..." }
```

---

### `POST /api/image/:id/transcode`
Trigger a full transcode for an existing image that has already been uploaded. Downloads the source from CloudFront/S3, runs all configured size variants, updates metadata.

**Response `200 OK`** — updated `ImageRecord` (same shape as upload response).

Returns `404` if the image or its source file is not found.

---

### `GET /api/category/list`
List configured categories and their size variants.

**Response `200 OK`**
```json
[
  {
    "name": "default",
    "sizes": [
      { "name": "small",  "width": 100, "height": 100, "mimeType": "image/jpeg" },
      { "name": "medium", "width": 200, "height": 200, "mimeType": "image/jpeg" },
      { "name": "large",  "width": 400, "height": 400, "mimeType": "image/png" }
    ]
  }
]
```

---

### `GET /test` / `GET /test/`
Serves the static HTML upload test page embedded in the binary.

---

## Configuration

Configuration is loaded in priority order (lowest → highest):

1. **`config.json`** — baked into the Docker image, provides defaults
2. **`/sandbox/config.json`** — volume-mounted override for secrets/environment-specific values (silently skipped if absent)
3. **Environment variables** — `__` as path separator, exact camelCase keys, e.g. `imageMetadata__storage=db`

All JSON key names are **camelCase** and match the original TypeScript service exactly.

### Full `config.json` reference

```json
{
  "server": {
    "port": 3000
  },
  "log": {
    "level": "info"
  },
  "imageMetadata": {
    "parentPath": "/metadata",
    "storage": "file"
  },
  "imagemagick": {
    "bin": "/usr/local/bin/magick"
  },
  "uploadDir": "/tmp",
  "s3": {
    "bucketName": "",
    "accessKeyId": "",
    "secretAccessKey": "",
    "region": "us-east-1"
  },
  "cloudfront": {
    "url": "",
    "expirySeconds": 10100,
    "keypairId": "",
    "privateKey": "",
    "privateKeyFile": ""
  },
  "db": {
    "host": "localhost",
    "port": 5432,
    "database": "image_service",
    "username": "image-service",
    "password": "image-service",
    "ssl": {
      "enabled": false,
      "verify": false,
      "caCertFile": ""
    },
    "minpool": 0,
    "maxpool": 10,
    "debug": false
  },
  "resize": {
    "processing": "deferred",
    "memoryLimit": 256,
    "mapLimit": 256,
    "sizeKeys": "small,medium,large",
    "original": {
      "mimeType": "image/png",
      "extension": ".png",
      "extraOpts": ""
    },
    "small": {
      "width": 100,
      "height": 100,
      "mimeType": "image/jpeg",
      "extension": ".jpg",
      "extraOpts": ""
    },
    "medium": {
      "width": 200,
      "height": 200,
      "mimeType": "image/jpeg",
      "extension": ".jpg",
      "extraOpts": ""
    },
    "large": {
      "width": 400,
      "height": 400,
      "mimeType": "image/png",
      "extension": ".png",
      "extraOpts": ""
    },
    "scalingSets": {}
  }
}
```

### Key config options

| Key | Description |
|---|---|
| `imageMetadata.storage` | `"file"` stores JSON on disk at `parentPath`; `"db"` uses PostgreSQL |
| `resize.processing` | `"deferred"` skips transcoding on upload unless `forceImmediateResize=true`; any other value transcodes immediately |
| `resize.sizeKeys` | Comma-separated list of size keys used for the `default` category |
| `resize.scalingSets` | Map of `{ "categoryName": "key1,key2" }` for non-default categories |
| `cloudfront.privateKey` | Inline PEM private key for CloudFront signing |
| `cloudfront.privateKeyFile` | Path to PEM file (used if `privateKey` is empty) |

### Environment variable examples

```bash
imageMetadata__storage=db
db__host=postgres
db__ssl__enabled=true
s3__bucketName=my-bucket
cloudfront__url=https://cdn.example.com
resize__processing=immediate
```

---

## Docker / Deployment

### Build and run

```bash
docker compose up --build
```

### docker-compose.yml overview

```yaml
services:
  image-service:
    build: .
    ports:
      - "3000:3000"
    environment:
      imageMetadata__storage: db
      db__host: postgres
    volumes:
      - ../../configs/image-service:/sandbox   # override config with secrets
    depends_on:
      postgres:
        condition: service_healthy

  postgres:
    image: postgres:18
    environment:
      POSTGRES_DB: image_service
      POSTGRES_USER: image-service
      POSTGRES_PASSWORD: image-service
```

Place production secrets (S3 credentials, CloudFront key, etc.) in `../../configs/image-service/config.json` — this directory is volume-mounted to `/sandbox` inside the container and deep-merged over the baked-in defaults at startup.

### Multi-stage Dockerfile

The build uses three stages:
1. **`imagemagick`** — compiles ImageMagick statically from source
2. **`builder`** — compiles the Rust binary using a stub `main.rs` to cache dependency compilation, then performs the real build
3. **`runtime`** — `debian:bookworm-slim` with the binary, ImageMagick, and config files

The runtime image does **not** use musl/Alpine; `debian:bookworm-slim` provides a glibc environment compatible with proc-macro crates.

---

## Metadata backends

### File backend (`imageMetadata.storage = "file"`)
Image records are stored as JSON files at `{parentPath}/{id}.json`. No database required. Suitable for development and single-instance deployments.

### PostgreSQL backend (`imageMetadata.storage = "db"`)
Migrations run automatically on startup. Schema:

```sql
CREATE TABLE images (
    id          UUID PRIMARY KEY,
    data        JSONB NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

The full `ImageRecord` is stored as JSONB, allowing schema-free evolution of the metadata fields.

---

## Development

### Prerequisites
- Rust 1.75+
- ImageMagick 7 (`magick` binary on PATH, or set `imagemagick.bin` in config)
- PostgreSQL 14+ (if using db backend)

### Run locally

```bash
# File backend (no database needed)
cargo run

# PostgreSQL backend
imageMetadata__storage=db db__host=localhost cargo run
```

### Compile checks without running

```bash
cargo check
```

### Test page

Visit `http://localhost:3000/test` for an interactive upload test page.
