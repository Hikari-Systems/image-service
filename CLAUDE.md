# CLAUDE.md — image-service-rs

Guidance for AI assistants working on this codebase.

---

## What this service does

Accepts image uploads, stores originals in S3, transcodes them to multiple size variants via ImageMagick, and serves signed CloudFront URLs for delivery. Metadata is stored either in PostgreSQL (JSONB) or on the filesystem (JSON files). This is a Rust/actix-web rewrite of the original TypeScript/Express service and must remain a drop-in replacement.

---

## Codebase map

```
src/
  main.rs                  — startup: config load, backend init, migrations, server bind
  config.rs                — AppConfig and all sub-structs; custom 3-layer loader
  state.rs                 — AppState struct (concrete, not dyn-based)
  routes/
    mod.rs                 — registers image + category route sets on ServiceConfig
    image.rs               — upload, get, redirect, signed URL, transcode endpoints
    category.rs            — GET /api/category/list
  models/
    image.rs               — ImageRecord, ImageDescriptor, ScaledImage, ImageBackend trait
    image_db.rs            — PostgreSQL backend (sqlx runtime queries, JSONB)
    image_file.rs          — filesystem backend (JSON files)
  services/
    s3.rs                  — S3Service: wraps aws-sdk-s3, reads file to bytes before upload
    cloudfront.rs          — CloudfrontService: RSA-SHA1 PKCS1v15 signed URL generation
    imagemagick.rs         — ImageMagickService: spawns `magick` as a subprocess
    downloader.rs          — DownloaderService: reqwest-based image fetcher
    svg_sanitiser.rs       — sanitise_svg: ammonia-based SVG cleaning
  helpers/
    transcode.rs           — process_image, transcode_image, scale_one, full_transcode
static/
  index.html               — test upload page (embedded via include_str! at compile time)
migrations/
  *.sql                    — sqlx migrations, run automatically on startup when db backend active
config.json                — default config values baked into the image
```

---

## Critical implementation decisions

### Config loading (`config.rs`)
**Do not use the `config` crate.** It normalises JSON keys to lowercase, which breaks `camelCase` field names like `bucketName`, `accessKeyId`, `caCertFile`. The custom loader in `AppConfig::load()` uses `serde_json::Value` for layering and `deep_merge()` for the sandbox override, then deserialises in one step.

Config priority (lowest → highest):
1. `config.json` in the working directory
2. `/sandbox/config.json` — always checked; silently ignored if absent; contains secrets/env-specific overrides
3. Env vars with `__` separator, exact camelCase path segments (e.g. `s3__bucketName=foo`)

### AppState (`state.rs`)
`AppState` is a **concrete struct**, not `web::Data<dyn Trait>`. The `Arc<dyn ImageBackend>` field is the only dynamic dispatch used; all other services are concrete. `web::Data<AppState>` extraction in handlers requires `AppState: !Sized` guard — concrete struct avoids this issue.

### Route registration (`routes/image.rs`)
Routes are registered directly on `web::ServiceConfig`, **never inside `web::scope("")`**. An empty scope matches all paths and swallows 404s, preventing other routes from matching.

Route order matters: `POST /api/image/{id}/transcode` must be registered **before** `POST /api/image/{category}` to prevent the `{category}` wildcard from consuming `transcode` as a category name.

### S3 uploads (`services/s3.rs`)
Files are read into memory with `tokio::fs::read()` before constructing `ByteStream`. Using `ByteStream::from_path()` with `behavior_version_latest()` causes silent hangs on PNG uploads — the SDK's streaming checksum computation deadlocks on certain file types/sizes.

### ImageMagick resizing (`helpers/transcode.rs`)
Size variants are processed **sequentially**, not via `join_all`. Parallel S3 uploads from concurrent ImageMagick outputs caused the third upload to hang indefinitely due to AWS SDK HTTP connection pool state.

Temp files for ImageMagick output use `tempfile::Builder::keep()` so the file persists until manually deleted after upload. Upload temp files (`NamedTempFile` in the route handler) auto-delete on drop.

### CloudFront signing (`services/cloudfront.rs`)
Uses `rsa` crate with `rsa::pkcs1v15::SigningKey<sha1::Sha1>`. PKCS1v15 with SHA-1 is required by CloudFront — RSA-PSS or SHA-256 will not work. The private key is loaded from either `cloudfront.privateKey` (inline PEM) or `cloudfront.privateKeyFile` (path to PEM file).

`cf_base64()` applies CloudFront's modified base64 alphabet (per AWS docs): `+` → `-`, `/` → `~`, `=` → `_`. Note: this is NOT standard URL-safe base64 (`/`→`_`, `=`→`~`) — the `/` and `=` mappings are the opposite of what you'd expect.

### sqlx queries (`models/image_db.rs`)
Use **runtime queries** (`query_as::<_, Row>(sql).bind(value)`) not compile-time macros (`query_as!`). The macro requires `DATABASE_URL` at compile time, which breaks Docker builds without a live database.

### Static file serving
`static/index.html` is embedded via `include_str!("../static/index.html")` and served from a handler. Do not use `actix-files` for this — the relative path `./static` resolves incorrectly in the container's working directory.

### TLS / OpenSSL
`reqwest` is configured with `default-features = false, features = ["rustls-tls", ...]`. This avoids the `openssl`/`pkg-config` dependency which is not available in the build stage. All TLS goes through `rustls`.

---

## Docker build notes

Three-stage build:
1. `imagemagick` — builds ImageMagick from source, produces a static binary at `/usr/local/bin/magick`
2. `builder` — Rust build stage. Uses stub `src/main.rs` (`fn main() {}`) + `cargo build --release --locked` to cache dependency compilation before copying real source. Real source is compiled with `touch src/main.rs && cargo build --release --locked`.
3. `runtime` — `debian:bookworm-slim` (not Alpine/musl). Copies binary, ImageMagick, config, migrations, and shared libs.

**Do not attempt static linking with `x86_64-unknown-linux-gnu` + `target-feature=+crt-static`.** Proc-macro crates (`actix-macros`) require the dynamic linker at build time and will fail. Static linking requires musl (`x86_64-unknown-linux-musl` target), which is intentionally avoided here.

**cargo-chef is not worth it** for this single-crate service. The stub `main.rs` pattern achieves the same dependency caching.

---

## Adding a new size variant

1. Add a new entry to `config.json` under `resize`, e.g.:
   ```json
   "xlarge": {
     "width": 800,
     "height": 800,
     "mimeType": "image/jpeg",
     "extension": ".jpg",
     "extraOpts": ""
   }
   ```
2. Add `xlarge` to `resize.sizeKeys` (or to a `scalingSets` entry for a specific category).
3. No code changes needed — `ResizeConfig` uses `#[serde(flatten)]` to capture named size entries.

## Adding a new category

Add an entry to `resize.scalingSets`:
```json
"scalingSets": {
  "avatar": "small,medium",
  "banner": "large,xlarge"
}
```
Upload to `POST /api/image/avatar` to use that size set.

---

## Common gotchas

- Config key names are **case-sensitive**. `bucketname` is not the same as `bucketName`.
- The `/sandbox/config.json` file is the intended mechanism for injecting secrets in production. Never put real credentials in the baked-in `config.json`.
- ImageMagick `memoryLimit` and `mapLimit` are in MiB. Keep them at 256+ to avoid resource contention when processing larger images.
- The `processing` field defaults to `"deferred"` — uploads return immediately without transcoding. Pass `?forceImmediateResize=true` or set `resize.processing` to any non-`"deferred"` value to transcode synchronously.
- `forceImmediateResize` also takes a comma-separated variant list (`?forceImmediateResize=original,small`) meaning "produce exactly these before responding", and `?defer=large` is the denylist form, subtracted from whatever the list and `resize.processing` selected. `defer` wins when a key appears in both. Both are parsed by `ResizeSelection` in `helpers/transcode.rs`; unknown keys are rejected with a `400` in the route handler, before anything is uploaded.
- Because an unrecognised value is now a size key rather than a silent "false", `forceImmediateResize` accepts the usual boolean spellings (`yes`/`y`/`on`/`1`/`all`, `no`/`n`/`off`/`0`/`none`). Previously only the literal `true` counted, so `?forceImmediateResize=yes` quietly meant *deferred*; it now means yes. Keep `is_truthy`/`is_falsey` in sync if the API grows another boolean-ish parameter.
- **`original` is a reserved variant name, not a size key.** It is a named field on `ResizeConfig`, consumed by serde before the `#[serde(flatten)] sizes` map, so `get_size("original")` is always `None`. It must never reach `scale_one`, which would abort the whole transcode. Never add it to `sizeKeys` or a `scalingSets` entry.
- **Partial transcodes merge, they do not replace.** `transcode_image` folds newly produced variants into `to_overwrite.resized_files` by size key and leaves `original_s3_path` untouched unless the original was produced in that pass. Any new write path must preserve this or a follow-up `POST /api/image/{id}/transcode?sizes=…` will erase the earlier pass's work.
- Database migrations run automatically when `imageMetadata.storage = "db"`. The migration table (`_sqlx_migrations`) is idempotent — running against an already-migrated database is safe.
