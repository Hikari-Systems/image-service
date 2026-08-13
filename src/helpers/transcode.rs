use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tempfile::Builder as TempBuilder;
use tokio::fs;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::models::image::{ImageDescriptor, ImageRecord, ScaledImage};
use crate::services::svg_sanitiser::sanitise_svg;
use crate::state::AppState;

/// Augment an `ImageRecord` with a pre-signed CloudFront URL for the original file.
pub async fn get_image_descriptor_with_download_url(
    img: Option<ImageRecord>,
    state: &AppState,
) -> Result<Option<ImageDescriptor>> {
    let Some(img) = img else {
        warn!("Image descriptor was null – aborting");
        return Ok(None);
    };

    if let Some(ref dl_path) = img.downloaded_s3_path {
        debug!("Using downloadedS3Path for image: {:?}", img.id);
        let signed_url = state.cf.get_signed_url(dl_path)?;
        return Ok(Some(ImageDescriptor {
            original_file_url: signed_url,
            record: img,
        }));
    }

    if let Some(ref src_url) = img.source_url {
        debug!("Using sourceUrl for image: {:?}", img.id);
        return Ok(Some(ImageDescriptor {
            original_file_url: src_url.clone(),
            record: img,
        }));
    }

    warn!("No usable URL found for image: {:?}", img.id);
    Ok(None)
}

/// Return the file extension from a path/filename string (including the dot).
/// Falls back to `.bin` if no extension is found.
pub fn extension_from_path(path: &str) -> String {
    match path.rfind('.') {
        Some(pos) if pos < path.len() - 1 => format!(".{}", &path[pos + 1..]),
        _ => ".bin".to_string(),
    }
}

/// The variant name reserved for the `-original` derivative. It is a named field on
/// `ResizeConfig`, not an entry in its flattened `sizes` map, so it can never be a
/// size key and must never reach `scale_one`.
pub const ORIGINAL_KEY: &str = "original";

/// Which half of the configured variants a request wants produced before it returns.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Immediate {
    /// Produce nothing now; leave it all for a later `POST /api/image/{id}/transcode`.
    None,
    /// Produce the original plus every size key configured for the category.
    All,
    /// Produce exactly these (may include `original`).
    Only(Vec<String>),
}

/// Resolved answer to "which variants happen inside this request?".
///
/// Two query parameters feed it: `forceImmediateResize` is an allowlist (or the
/// historic `true`/`false`), and `defer` is a denylist subtracted from whatever the
/// allowlist and `resize.processing` produced. `defer` is the last word — naming a
/// key in both is a request to defer it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResizeSelection {
    immediate: Immediate,
    deferred: Vec<String>,
}

impl ResizeSelection {
    /// Resolve the two query parameters.
    ///
    /// `default_immediate` is what an absent (or `true`/`false`) allowlist falls back
    /// to: the upload route derives it from `resize.processing`, while the transcode
    /// endpoint passes `true` because it has always transcoded unconditionally. An
    /// explicit list always wins over that fallback, in both directions.
    pub fn parse(force: Option<&str>, defer: Option<&str>, default_immediate: bool) -> Self {
        let immediate = match force.map(str::trim) {
            None | Some("") => {
                if default_immediate { Immediate::All } else { Immediate::None }
            }
            Some(v) if is_truthy(v) => Immediate::All,
            // A falsey value historically did not disable anything when the service
            // was configured for immediate processing — the old check was an `||`.
            Some(v) if is_falsey(v) => {
                if default_immediate { Immediate::All } else { Immediate::None }
            }
            Some(v) => Immediate::Only(split_keys(v)),
        };

        Self { immediate, deferred: defer.map(split_keys).unwrap_or_default() }
    }

    /// Every key named in either parameter, for validation against the config.
    pub fn requested_keys(&self) -> Vec<&str> {
        let mut keys: Vec<&str> = match &self.immediate {
            Immediate::Only(keys) => keys.iter().map(String::as_str).collect(),
            _ => Vec::new(),
        };
        keys.extend(self.deferred.iter().map(String::as_str));
        keys
    }

    /// Should this variant be produced in this pass?
    pub fn wants(&self, key: &str) -> bool {
        if self.deferred.iter().any(|d| d == key) {
            return false;
        }
        match &self.immediate {
            Immediate::None => false,
            Immediate::All => true,
            Immediate::Only(keys) => keys.iter().any(|k| k == key),
        }
    }

    /// True when nothing at all would be produced, so the transcode can be skipped
    /// outright. `size_keys` is the category's configured set.
    fn wants_nothing(&self, size_keys: &[String]) -> bool {
        !self.wants(ORIGINAL_KEY) && !size_keys.iter().any(|k| self.wants(k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZES: [&str; 3] = ["small", "medium", "large"];

    /// Every variant this selection would produce, original included, in config order.
    fn produced(sel: &ResizeSelection) -> Vec<&'static str> {
        std::iter::once(ORIGINAL_KEY)
            .chain(SIZES)
            .filter(|k| sel.wants(k))
            .collect()
    }

    #[test]
    fn absent_param_follows_configured_mode() {
        assert!(produced(&ResizeSelection::parse(None, None, false)).is_empty());
        assert_eq!(
            produced(&ResizeSelection::parse(None, None, true)),
            ["original", "small", "medium", "large"]
        );
    }

    #[test]
    fn boolean_spellings_are_understood() {
        for yes in ["TRUE", "true", "yes", "Y", "on", "1", "all"] {
            assert_eq!(
                produced(&ResizeSelection::parse(Some(yes), None, false)),
                ["original", "small", "medium", "large"],
                "{yes} should mean everything"
            );
        }
        for no in ["false", "NO", "n", "off", "0", "none"] {
            assert!(
                produced(&ResizeSelection::parse(Some(no), None, false)).is_empty(),
                "{no} should mean nothing"
            );
        }
    }

    #[test]
    fn true_and_false_keep_their_historic_meaning() {
        assert_eq!(
            produced(&ResizeSelection::parse(Some("TRUE"), None, false)),
            ["original", "small", "medium", "large"]
        );
        assert!(produced(&ResizeSelection::parse(Some("false"), None, false)).is_empty());
        // The old check was an `||`, so `false` never disabled a service configured
        // for immediate processing.
        assert_eq!(
            produced(&ResizeSelection::parse(Some("false"), None, true)),
            ["original", "small", "medium", "large"]
        );
    }

    #[test]
    fn explicit_list_wins_over_configured_mode() {
        assert_eq!(
            produced(&ResizeSelection::parse(Some("original, small"), None, false)),
            ["original", "small"]
        );
        assert_eq!(
            produced(&ResizeSelection::parse(Some("small,large"), None, true)),
            ["small", "large"]
        );
    }

    #[test]
    fn defer_subtracts_and_has_the_last_word() {
        assert_eq!(
            produced(&ResizeSelection::parse(Some("true"), Some("large"), false)),
            ["original", "small", "medium"]
        );
        assert_eq!(
            produced(&ResizeSelection::parse(None, Some("original"), true)),
            ["small", "medium", "large"]
        );
        // Named in both: deferring is the more specific instruction.
        assert_eq!(
            produced(&ResizeSelection::parse(Some("small,large"), Some("large"), false)),
            ["small"]
        );
    }

    #[test]
    fn wants_nothing_detects_an_empty_selection() {
        let keys: Vec<String> = SIZES.iter().map(|s| s.to_string()).collect();
        let empty = ResizeSelection::parse(Some("small"), Some("small"), false);
        assert!(empty.wants_nothing(&keys));
        assert!(!ResizeSelection::parse(Some("small"), None, false).wants_nothing(&keys));
        // Only the original still counts as work to do.
        assert!(!ResizeSelection::parse(Some("original"), None, false).wants_nothing(&keys));
    }

    #[test]
    fn requested_keys_covers_both_params_for_validation() {
        let sel = ResizeSelection::parse(Some("small, bogus"), Some("nope"), false);
        assert_eq!(sel.requested_keys(), ["small", "bogus", "nope"]);
        // `true` names no keys, so there is nothing to validate.
        assert!(ResizeSelection::parse(Some("true"), None, false)
            .requested_keys()
            .is_empty());
    }
}

/// Boolean spellings the parameter accepts, so that a caller writing `yes` or `1`
/// gets what they plainly meant rather than a "no such size key" rejection. Only the
/// literal `true` used to count, which quietly made every other spelling mean *false*;
/// `yes` now means yes.
fn is_truthy(v: &str) -> bool {
    ["true", "yes", "y", "on", "1", "all"]
        .iter()
        .any(|w| v.eq_ignore_ascii_case(w))
}

fn is_falsey(v: &str) -> bool {
    ["false", "no", "n", "off", "0", "none"]
        .iter()
        .any(|w| v.eq_ignore_ascii_case(w))
}

/// Split a comma-separated parameter into trimmed, non-empty keys.
fn split_keys(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Upload the local file to S3, upsert image metadata, then produce whichever
/// variants `selection` asks for.
pub async fn process_image(
    local_source_path: &Path,
    source_extension: &str,
    source_mime_type: &str,
    category: &str,
    url: Option<&str>,
    selection: &ResizeSelection,
    to_overwrite: Option<ImageRecord>,
    state: &AppState,
) -> Result<ImageRecord> {
    let base = to_overwrite.unwrap_or_default();
    let id = base.id.unwrap_or_else(Uuid::new_v4);

    let s3_path = format!("{}-{}{}", category, id, source_extension);
    info!("S3 upload start: {} → {}", local_source_path.display(), s3_path);
    state
        .s3
        .save(local_source_path, &s3_path, source_mime_type)
        .await
        .context("Failed to upload original to S3")?;
    info!("S3 upload done: {}", s3_path);

    info!("DB upsert: id={}", id);
    let uploaded = state
        .backend
        .upsert(ImageRecord {
            id: Some(id),
            downloaded_s3_path: Some(s3_path),
            category: Some(category.to_string()),
            source_url: url.map(|u| u.to_string()),
            ..base
        })
        .await
        .context("Failed to upsert image after upload")?;
    info!("DB upsert done: id={:?}", uploaded.id);

    let size_keys = state
        .config
        .resize
        .size_keys_for_category(&category.to_lowercase());
    if selection.wants_nothing(&size_keys) {
        return Ok(uploaded);
    }

    info!("Transcode start: id={:?}", uploaded.id);
    let result = transcode_image(
        local_source_path,
        uploaded,
        source_mime_type,
        source_extension,
        selection,
        state,
    )
    .await;
    info!("Transcode done: id={}", id);
    result
}

/// Formats that can be served to a browser exactly as uploaded. For these the
/// "original" derivative is a **copy** of the source, not a re-encode.
///
/// Re-encoding one of these is pure loss on every axis: it costs CPU, it discards
/// quality (a JPEG re-encoded to JPEG is generation loss), and converting to the
/// configured target can *drop information the source carried* — a PNG or WebP
/// with an alpha channel flattened into JPEG loses transparency irrecoverably.
/// Copying preserves the bytes, so alpha, colour profile and EXIF all survive.
fn servable_as_original(mime: &str) -> bool {
    matches!(
        mime,
        "image/jpeg" | "image/jpg" | "image/png" | "image/webp" | "image/gif" | "image/avif"
    )
}

/// Produce the variants `selection` asks for: the original derivative and/or any of
/// the size variants configured for the category. Variants left out are untouched —
/// whatever an earlier pass stored for them survives into the updated record.
async fn transcode_image(
    local_source_path: &Path,
    to_overwrite: ImageRecord,
    local_file_content_type: &str,
    source_extension: &str,
    selection: &ResizeSelection,
    state: &AppState,
) -> Result<ImageRecord> {
    debug!(
        "transcode_image: src={:?} id={:?}",
        local_source_path, to_overwrite.id
    );

    let cfg = &state.config;

    // The original keeps the source's own format when that format is directly
    // servable; `resize.original` (PNG by default) is the fallback for formats a
    // browser cannot render — HEIC, TIFF, BMP — where a lossless, alpha-capable
    // target is exactly what you want.
    let copy_source_as_original =
        servable_as_original(local_file_content_type) && !source_extension.is_empty();

    let (orig_ext, orig_mime) = if copy_source_as_original {
        (
            source_extension.to_string(),
            local_file_content_type.to_string(),
        )
    } else {
        (
            cfg.resize
                .original
                .extension
                .as_deref()
                .unwrap_or(".png")
                .to_string(),
            cfg.resize
                .original
                .mime_type
                .as_deref()
                .unwrap_or("image/png")
                .to_string(),
        )
    };

    let category = to_overwrite
        .category
        .as_deref()
        .filter(|c| !c.is_empty())
        .unwrap_or("image")
        .to_lowercase();

    let id = to_overwrite.id.unwrap_or_else(Uuid::new_v4);
    let orig_dest_s3_path = format!("{}-{}-original{}", category, id, orig_ext);

    // Process the original file
    let do_original = selection.wants(ORIGINAL_KEY);
    if !do_original {
        info!("Skipping original for id={} (deferred by request)", id);
    } else if local_file_content_type == "image/svg+xml" {
        info!("SVG sanitise start");
        let sanitised = sanitise_svg(local_source_path)
            .await
            .context("SVG sanitisation failed")?;
        info!("SVG sanitise done, uploading original");
        state
            .s3
            .save(&sanitised, &orig_dest_s3_path, "image/svg+xml")
            .await?;
        let _ = fs::remove_file(&sanitised).await;
        info!("Original SVG uploaded: {}", orig_dest_s3_path);
    } else if copy_source_as_original {
        // Already servable — upload the source bytes unchanged. This is the common
        // case (every phone photo, every PNG avatar) and it is the whole cost saving:
        // on a 12 MP JPEG the re-encode this replaces measured 7.4s and produced a
        // 16.8 MB PNG, which is 80% of the entire transcode. Uploads are synchronous
        // (callers pass ?forceImmediateResize=true), so that landed inside the request
        // and pushed multi-photo uploads past the 30s proxy timeout as browser 504s.
        info!(
            "Original is already servable ({}), copying source unchanged: {}",
            local_file_content_type, orig_dest_s3_path
        );
        state
            .s3
            .save(local_source_path, &orig_dest_s3_path, &orig_mime)
            .await?;
        info!("Original uploaded: {}", orig_dest_s3_path);
    } else {
        let orig_extra = cfg.resize.original.extra_opts.as_str();
        info!("ImageMagick original start: src={}", local_source_path.display());
        let orig_resized = state
            .magick
            .resize_image(local_source_path, -1, -1, orig_extra, &orig_ext)
            .await
            .context("ImageMagick failed for original")?;
        info!("ImageMagick original done, uploading: {}", orig_dest_s3_path);
        state
            .s3
            .save(&orig_resized, &orig_dest_s3_path, &orig_mime)
            .await?;
        let _ = fs::remove_file(&orig_resized).await;
        info!("Original uploaded: {}", orig_dest_s3_path);
    }

    // Resize to the requested sizes sequentially to avoid S3 connection pool contention
    let configured = cfg.resize.size_keys_for_category(&category);
    let size_keys: Vec<String> = configured
        .iter()
        .filter(|k| selection.wants(k))
        .cloned()
        .collect();
    info!(
        "Scaling to {} of {} configured size(s): {:?}",
        size_keys.len(),
        configured.len(),
        size_keys
    );
    let mut scaled: Vec<ScaledImage> = Vec::new();
    for size_key in &size_keys {
        let si = scale_one(local_source_path.to_path_buf(), size_key.clone(), id, category.clone(), state)
            .await
            .with_context(|| format!("Size variant '{}' failed to transcode", size_key))?;
        scaled.push(si);
    }

    // Merge rather than replace: a pass that produced only some variants must not
    // erase the ones an earlier pass stored.
    let mut resized_files = to_overwrite.resized_files.unwrap_or_default();
    for si in scaled {
        match resized_files.iter_mut().find(|existing| existing.size == si.size) {
            Some(existing) => *existing = si,
            None => resized_files.push(si),
        }
    }

    let updated = ImageRecord {
        id: Some(id),
        category: Some(category),
        original_s3_path: if do_original {
            Some(orig_dest_s3_path)
        } else {
            to_overwrite.original_s3_path
        },
        resized_files: Some(resized_files),
        avoid_resize_until: to_overwrite.avoid_resize_until,
        source_url: to_overwrite.source_url,
        created_at: to_overwrite.created_at,
        downloaded_s3_path: to_overwrite.downloaded_s3_path,
    };

    state
        .backend
        .upsert(updated)
        .await
        .context("Failed to upsert image after transcoding")
}

/// Produce one scaled variant, upload to S3, return the ScaledImage descriptor.
async fn scale_one(
    source: PathBuf,
    size_key: String,
    id: Uuid,
    category: String,
    state: &AppState,
) -> Result<ScaledImage> {
    let cfg = &state.config;
    let size_cfg = cfg
        .resize
        .get_size(&size_key)
        .with_context(|| format!("No size config found for key '{}'", size_key))?;

    let ext = size_cfg.extension.as_deref().unwrap_or(".jpg").to_string();
    let mime = size_cfg.mime_type.as_deref().unwrap_or("image/jpeg").to_string();
    let w = size_cfg.width.unwrap_or(0);
    let h = size_cfg.height.unwrap_or(0);
    let extra_opts = size_cfg.extra_opts.clone();

    info!("scale_one[{}]: ImageMagick start ({}x{})", size_key, w, h);
    let scaled_path = state
        .magick
        .resize_image(&source, w, h, &extra_opts, &ext)
        .await
        .with_context(|| format!("ImageMagick failed for size '{}'", size_key))?;
    info!("scale_one[{}]: ImageMagick done → {:?}", size_key, scaled_path);

    let s3_key = format!("{}-{}-{}{}", category, id, size_key, ext);
    info!("scale_one[{}]: S3 upload start → {}", size_key, s3_key);
    state
        .s3
        .save(&scaled_path, &s3_key, &mime)
        .await
        .with_context(|| format!("S3 upload failed for size '{}'", size_key))?;
    info!("scale_one[{}]: S3 upload done", size_key);

    let _ = fs::remove_file(&scaled_path).await;

    Ok(ScaledImage {
        size: size_key,
        s3_path: s3_key,
    })
}

/// Trigger a transcode for an image that already has a downloaded or source path.
/// Downloads the file if necessary, then calls `transcode_image`. The selection can
/// be every variant, or a narrower one to complete only what an earlier pass deferred.
pub async fn full_transcode(
    image_item: ImageRecord,
    selection: &ResizeSelection,
    state: &AppState,
) -> Result<ImageRecord> {
    if let Some(ref dl_path) = image_item.downloaded_s3_path {
        let ext = extension_from_path(dl_path);
        let dl_url = state
            .cf
            .get_signed_url(dl_path)
            .context("Failed to sign CloudFront URL for transcode download")?;

        let tmp = TempBuilder::new()
            .suffix(&ext)
            .tempfile()
            .context("Failed to create temp file for transcode download")?;
        let (_, tmp_path) = tmp.keep()?;

        let downloaded = state
            .downloader
            .download_image(&dl_url, &tmp_path)
            .await
            .context("Failed to download image for transcoding")?;

        let result = transcode_image(
            &downloaded.local_path,
            image_item,
            &downloaded.mime_type,
            &ext,
            selection,
            state,
        )
        .await;
        let _ = fs::remove_file(&tmp_path).await;
        return result;
    }

    if let Some(ref src_url) = image_item.source_url.clone() {
        let ext = {
            let parsed = url::Url::parse(src_url)
                .unwrap_or_else(|_| url::Url::parse("http://x/").unwrap());
            extension_from_path(parsed.path())
        };
        let cat = image_item
            .category
            .as_deref()
            .filter(|c| !c.is_empty())
            .unwrap_or("image")
            .to_string();
        let id = image_item.id.unwrap_or_else(Uuid::new_v4);

        let tmp = TempBuilder::new()
            .suffix(&ext)
            .tempfile()
            .context("Failed to create temp file for source download")?;
        let (_, tmp_path) = tmp.keep()?;

        let downloaded = state
            .downloader
            .download_image(src_url, &tmp_path)
            .await
            .context("Failed to download image from source URL")?;

        let s3_dl_path = format!("{}-{}{}", cat, id, ext);
        state
            .s3
            .save(&downloaded.local_path, &s3_dl_path, &downloaded.mime_type)
            .await?;

        let with_dl = ImageRecord {
            downloaded_s3_path: Some(s3_dl_path),
            ..image_item
        };

        let result = transcode_image(
            &downloaded.local_path,
            with_dl,
            &downloaded.mime_type,
            &ext,
            selection,
            state,
        )
        .await;
        let _ = fs::remove_file(&tmp_path).await;
        return result;
    }

    Err(anyhow::anyhow!(
        "No downloadedS3Path and no sourceUrl for image {:?}",
        image_item.id
    ))
}
