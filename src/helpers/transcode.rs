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

/// Upload the local file to S3, upsert image metadata, then optionally transcode.
pub async fn process_image(
    local_source_path: &Path,
    source_extension: &str,
    source_mime_type: &str,
    category: &str,
    url: Option<&str>,
    force_immediate_resize: bool,
    to_overwrite: Option<ImageRecord>,
    state: &AppState,
) -> Result<ImageRecord> {
    let do_transcode = force_immediate_resize
        || state.config.resize.processing.trim() != "deferred";

    do_process_image(
        local_source_path,
        source_extension,
        source_mime_type,
        category,
        url,
        do_transcode,
        to_overwrite,
        state,
    )
    .await
}

async fn do_process_image(
    local_source_path: &Path,
    source_extension: &str,
    source_mime_type: &str,
    category: &str,
    url: Option<&str>,
    do_transcode: bool,
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

    if do_transcode {
        info!("Transcode start: id={:?}", uploaded.id);
        let result = transcode_image(local_source_path, uploaded, source_mime_type, state).await;
        info!("Transcode done: id={}", id);
        result
    } else {
        Ok(uploaded)
    }
}

/// Re-encode the original and produce all configured size variants.
async fn transcode_image(
    local_source_path: &Path,
    to_overwrite: ImageRecord,
    local_file_content_type: &str,
    state: &AppState,
) -> Result<ImageRecord> {
    debug!(
        "transcode_image: src={:?} id={:?}",
        local_source_path, to_overwrite.id
    );

    let cfg = &state.config;
    let orig_ext = cfg
        .resize
        .original
        .extension
        .as_deref()
        .unwrap_or(".png")
        .to_string();
    let orig_mime = cfg
        .resize
        .original
        .mime_type
        .as_deref()
        .unwrap_or("image/png")
        .to_string();

    let category = to_overwrite
        .category
        .as_deref()
        .filter(|c| !c.is_empty())
        .unwrap_or("image")
        .to_lowercase();

    let id = to_overwrite.id.unwrap_or_else(Uuid::new_v4);
    let orig_dest_s3_path = format!("{}-{}-original{}", category, id, orig_ext);

    // Process the original file
    if local_file_content_type == "image/svg+xml" {
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

    // Resize to all configured sizes sequentially to avoid S3 connection pool contention
    let size_keys = cfg.resize.size_keys_for_category(&category);
    info!("Scaling to {} size(s): {:?}", size_keys.len(), size_keys);
    let mut scaled: Vec<ScaledImage> = Vec::new();
    for size_key in &size_keys {
        let si = scale_one(local_source_path.to_path_buf(), size_key.clone(), id, category.clone(), state)
            .await
            .with_context(|| format!("Size variant '{}' failed to transcode", size_key))?;
        scaled.push(si);
    }

    let updated = ImageRecord {
        id: Some(id),
        category: Some(category),
        original_s3_path: Some(orig_dest_s3_path),
        resized_files: Some(scaled),
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

/// Trigger a full transcode for an image that already has a downloaded or source path.
/// Downloads the file if necessary, then calls `transcode_image`.
pub async fn full_transcode(
    image_item: ImageRecord,
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
