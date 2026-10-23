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
| `forceImmediateResize` | bool or list | If truthy (`true`, `yes`, `y`, `on`, `1`, `all` — any case), transcode everything synchronously before responding. If a comma-separated list of variant names (e.g. `original,small`), produce only those before responding. Falsey values (`false`, `no`, `n`, `off`, `0`, `none`) and absence follow the `resize.processing` config. |
| `defer` | list | Comma-separated variant names to *not* produce during the upload, e.g. `defer=large`. Subtracted from whatever `forceImmediateResize` and `resize.processing` selected, and wins over both. |

Variant names are the size keys configured for the category, plus `original` for the
`-original.*` derivative. An unknown name is a `400`. The raw source upload
(`downloadedS3Path`) always happens regardless of either parameter.

Whatever is not produced during the upload is left to a later
`POST /api/image/:id/transcode`, which merges its output into the existing record
rather than replacing it. So `?forceImmediateResize=original,small` returns as soon as
the thumbnail a page needs is ready, and the remaining sizes can be filled in
afterwards with `POST /api/image/:id/transcode?sizes=medium,large`.

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
Trigger a transcode for an existing image that has already been uploaded. Downloads the source from CloudFront/S3, runs the requested size variants, updates metadata. With no query params it runs every configured variant, as before.

**Query params**
| Param | Type | Description |
|---|---|---|
| `sizes` | list | Comma-separated variant names to produce, e.g. `medium,large`. Absent means all of them. |
| `defer` | list | Comma-separated variant names to skip. Subtracted from `sizes`, and wins over it. |

Names are the size keys configured for the category, plus `original`. An unknown name is a `400`.

**Response `200 OK`** — updated `ImageRecord` (same shape as upload response).

Variants produced here are merged into the existing record: `resizedFiles` entries are
added or replaced by size key, and `originalS3Path` is left alone unless `original` was
produced in this pass. That makes it safe to complete an upload piecemeal.

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
| `resize.processing` | `"deferred"` skips transcoding on upload unless `forceImmediateResize` asks for it; any other value transcodes immediately. Either way an explicit variant list on the request wins. |
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

  minio:          # local S3 stand-in — see "Local S3 with MinIO" below
    image: minio/minio:latest
    ports: ["9000:9000", "9001:9001"]

  minio-init:     # creates the bucket, opens it for anonymous GET, exits
    image: minio/mc:latest
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

### Local S3 with MinIO

Real uploads need a bucket, so `docker-compose.yml` includes MinIO as an S3 stand-in
plus a one-shot `minio-init` that creates the bucket and opens it for anonymous GET.
With CloudFront signing switched off (no `cloudfront.keypairId`), the URLs the API
returns point straight at MinIO — which is what lets `POST /api/image/:id/transcode`
re-download the source locally.

```bash
docker compose up -d minio minio-init     # MinIO on :9000, console on :9001
source scripts/local-env.sh               # env for the host process
cargo run
```

`scripts/local-env.sh` points the service at MinIO, keeps metadata on disk under
`.local/metadata`, and picks up `magick` or IM6's `convert`, whichever is installed.
Objects are then browsable at `http://localhost:9000/image-service-local/<key>`
(login `minioadmin` / `minioadmin` for the console).

```bash
./scripts/local-smoke-test.sh             # end-to-end check of variant selection
```

The smoke test asserts on both the returned record and what actually landed in the
bucket, including that a deferred variant is genuinely absent and that a follow-up
transcode merges into the record rather than replacing it. Tear down with
`docker compose down -v`.

`docker compose up` also runs the service itself against MinIO. In that setup
`cloudfront.url` uses the in-network `minio` hostname, so swap it for `localhost` in
any URL the API hands back before fetching it from the host.

### Test page

Visit `http://localhost:3000/test` for an interactive upload test page.
