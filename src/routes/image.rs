use actix_multipart::Multipart;
use actix_web::{web, HttpResponse};
use futures_util::StreamExt;
use std::io::Write;
use tempfile::Builder as TempBuilder;
use tracing::{debug, error};

use crate::helpers::transcode::{
    extension_from_path, full_transcode, get_image_descriptor_with_download_url, process_image,
    ResizeSelection, ORIGINAL_KEY,
};
use crate::models::image::ScaledImage;
use crate::state::AppState;

pub fn configure(cfg: &mut web::ServiceConfig) {
    // Transcode must come before /{id} to avoid being swallowed by the wildcard segment.
    cfg.route("/api/image/{id}/transcode", web::post().to(transcode_handler))
        .route("/api/image/r/{id}/{size}", web::get().to(get_redirect))
        .route("/api/image/s/{id}/{size}", web::get().to(get_signed_url))
        .route("/api/image/{id}", web::get().to(get_image))
        .route("/api/image/{category}", web::post().to(upload_image));
}

/// GET /api/image/:id — return image descriptor JSON with a pre-signed original URL.
async fn get_image(
    path: web::Path<String>,
    state: web::Data<AppState>,
) -> HttpResponse {
    let id = path.into_inner();
    debug!("Get by id: {}", id);
    match state.backend.get(&id).await {
        Err(e) => {
            error!("Error getting image id={}: {}", id, e);
            HttpResponse::InternalServerError().finish()
        }
        Ok(record) => {
            match get_image_descriptor_with_download_url(record, &state).await {
                Err(e) => {
                    error!("Error building descriptor for id={}: {}", id, e);
                    HttpResponse::InternalServerError().finish()
                }
                Ok(None) => HttpResponse::NotFound().body("Not found"),
                Ok(Some(desc)) => HttpResponse::Ok().json(desc),
            }
        }
    }
}

/// GET /api/image/r/:id/:size — redirect to the signed CloudFront URL for the requested size.
async fn get_redirect(
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
) -> HttpResponse {
    let (id, size) = path.into_inner();
    debug!("Get redirect for image {} size {}", id, size);

    match resolve_image_url(&id, &size, &state).await {
        Err(e) => {
            error!("Error getting image id={}: {}", id, e);
            HttpResponse::InternalServerError().finish()
        }
        Ok(None) => HttpResponse::NotFound().body(format!("Image not found: {}", id)),
        Ok(Some(url)) => HttpResponse::Found()
            .append_header(("Location", url))
            .finish(),
    }
}

/// GET /api/image/s/:id/:size — return JSON `{"url": "..."}` for the requested size.
async fn get_signed_url(
    path: web::Path<(String, String)>,
    state: web::Data<AppState>,
) -> HttpResponse {
    let (id, size) = path.into_inner();
    debug!("Get signed url for image {} size {}", id, size);

    match resolve_image_url(&id, &size, &state).await {
        Err(e) => {
            error!("Error getting image id={}: {}", id, e);
            HttpResponse::InternalServerError().finish()
        }
        Ok(None) => HttpResponse::NotFound().body(format!("Image not found: {}", id)),
        Ok(Some(url)) => HttpResponse::Ok().json(serde_json::json!({ "url": url })),
    }
}

/// Shared logic: find the best available URL for an image at a requested size.
/// Returns None if the image record doesn't exist.
async fn resolve_image_url(
    id: &str,
    size: &str,
    state: &AppState,
) -> anyhow::Result<Option<String>> {
    let Some(image) = state.backend.get(id).await? else {
        debug!("Image not found: {}", id);
        return Ok(None);
    };

    // 1. Try the exact requested size
    let scaled: Vec<&ScaledImage> = image
        .resized_files
        .as_deref()
        .unwrap_or(&[])
        .iter()
        .filter(|x| x.size == size)
        .collect();

    if let Some(sc) = scaled.first() {
        debug!("Resized version found for size {} in {:?}", size, image.id);
        let signed = state.cf.get_signed_url(&sc.s3_path)?;
        return Ok(Some(signed));
    }

    // 2. Fall back to originalS3Path
    if let Some(ref orig) = image.original_s3_path {
        debug!("No resized version for size {} – serving original for {:?}", size, image.id);
        let signed = state.cf.get_signed_url(orig)?;
        return Ok(Some(signed));
    }

    // 3. Fall back to downloadedS3Path
    if let Some(ref dl) = image.downloaded_s3_path {
        debug!("No processed version for size {} – serving downloaded for {:?}", size, image.id);
        let signed = state.cf.get_signed_url(dl)?;
        return Ok(Some(signed));
    }

    // 4. Fall back to sourceUrl
    if let Some(ref src) = image.source_url {
        if !src.is_empty() {
            debug!("No downloaded version – serving source URL for {:?}", image.id);
            return Ok(Some(src.clone()));
        }
    }

    Err(anyhow::anyhow!("No usable image data for image {}", id))
}

/// Resolve the variant-selection query parameters, rejecting any key the config does
/// not define. Validating up front means a typo is a 400 rather than a 500 raised
/// halfway through, after the source has already been uploaded to S3.
fn selection_from_query(
    force: Option<&String>,
    defer: Option<&String>,
    default_immediate: bool,
    state: &AppState,
) -> Result<ResizeSelection, String> {
    let selection = ResizeSelection::parse(
        force.map(String::as_str),
        defer.map(String::as_str),
        default_immediate,
    );

    for key in selection.requested_keys() {
        if key != ORIGINAL_KEY && state.config.resize.get_size(key).is_none() {
            return Err(format!("Unknown size key: {}", key));
        }
    }

    Ok(selection)
}

/// POST /api/image/:id/transcode — transcode an existing image. Accepts `?sizes=` and
/// `?defer=` to produce a subset; with neither, every variant is produced as before.
async fn transcode_handler(
    path: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
    state: web::Data<AppState>,
) -> HttpResponse {
    let id = path.into_inner();

    // This endpoint has always transcoded regardless of `resize.processing`, so an
    // absent `sizes` means everything.
    let selection = match selection_from_query(query.get("sizes"), query.get("defer"), true, &state)
    {
        Ok(s) => s,
        Err(msg) => {
            error!("Transcode rejected for id={}: {}", id, msg);
            return HttpResponse::BadRequest().body(msg);
        }
    };
    debug!("Transcode requested for id={} selection={:?}", id, selection);

    match state.backend.get(&id).await {
        Err(e) => {
            error!("Error getting image id={}: {}", id, e);
            HttpResponse::InternalServerError().finish()
        }
        Ok(None) => HttpResponse::NotFound().body(format!("Image not found: {}", id)),
        Ok(Some(img)) => {
            if img.downloaded_s3_path.is_none() {
                return HttpResponse::NotFound().finish();
            }
            match full_transcode(img, &selection, &state).await {
                Ok(result) => HttpResponse::Ok().json(result),
                Err(e) => {
                    error!("Error transcoding image id={}: {}", id, e);
                    HttpResponse::InternalServerError().finish()
                }
            }
        }
    }
}

/// POST /api/image/:category — accept a multipart upload, save to S3, optionally transcode.
async fn upload_image(
    path: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
    mut payload: Multipart,
    state: web::Data<AppState>,
) -> HttpResponse {
    let category = path.into_inner();

    let default_immediate = state.config.resize.processing.trim() != "deferred";
    let selection = match selection_from_query(
        query.get("forceImmediateResize"),
        query.get("defer"),
        default_immediate,
        &state,
    ) {
        Ok(s) => s,
        Err(msg) => {
            error!("Upload rejected for category={}: {}", category, msg);
            return HttpResponse::BadRequest().body(msg);
        }
    };

    // Read the first multipart field named "image"
    let mut found_file: Option<(tempfile::NamedTempFile, String, String)> = None;

    while let Some(item) = payload.next().await {
        let mut field = match item {
            Ok(f) => f,
            Err(e) => {
                error!("Multipart error: {}", e);
                return HttpResponse::BadRequest().body("Multipart error");
            }
        };

        // Extract field metadata before we consume the stream
        let field_name = field
            .content_disposition()
            .and_then(|cd| cd.get_name())
            .unwrap_or("")
            .to_string();

        if field_name != "image" {
            continue;
        }

        let original_filename = field
            .content_disposition()
            .and_then(|cd| cd.get_filename())
            .unwrap_or("")
            .to_string();

        let content_type = field
            .content_type()
            .map(|m| m.to_string())
            .unwrap_or_else(|| "application/octet-stream".to_string());

        let ext = extension_from_path(&original_filename);

        // Write field bytes to a named temp file
        let tmp = match TempBuilder::new().suffix(&ext).tempfile() {
            Ok(t) => t,
            Err(e) => {
                error!("Failed to create temp file: {}", e);
                return HttpResponse::InternalServerError().finish();
            }
        };

        let mut written = tmp;
        while let Some(chunk) = field.next().await {
            let data = match chunk {
                Ok(d) => d,
                Err(e) => {
                    error!("Error reading multipart chunk: {}", e);
                    return HttpResponse::BadRequest().body("Error reading upload");
                }
            };
            if let Err(e) = written.write_all(&data) {
                error!("Error writing temp file: {}", e);
                return HttpResponse::InternalServerError().finish();
            }
        }

        found_file = Some((written, content_type, ext));
        break;
    }

    let Some((tmp_file, content_type, ext)) = found_file else {
        error!("No image file supplied");
        return HttpResponse::BadRequest().body("No image file supplied");
    };

    let tmp_path = tmp_file.path().to_path_buf();

    debug!(
        "Image uploaded: path={:?} mime={} selection={:?}",
        tmp_path, content_type, selection
    );

    match process_image(
        &tmp_path,
        &ext,
        &content_type,
        &category,
        None,
        &selection,
        None,
        &state,
    )
    .await
    {
        Ok(record) => {
            // tmp_file drops here, auto-deleting the temp file
            HttpResponse::Created().json(record)
        }
        Err(e) => {
            error!("Error processing uploaded image: {}", e);
            HttpResponse::InternalServerError().finish()
        }
    }
}
