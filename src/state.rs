use std::sync::Arc;

use crate::config::AppConfig;
use crate::models::image::ImageBackend;
use crate::services::cloudfront::CloudfrontService;
use crate::services::downloader::DownloaderService;
use crate::services::imagemagick::ImageMagickService;
use crate::services::s3::S3Service;

/// All shared application state, passed to route handlers via `web::Data<AppState>`.
pub struct AppState {
    pub config: AppConfig,
    pub backend: Arc<dyn ImageBackend>,
    pub s3: S3Service,
    pub cf: CloudfrontService,
    pub magick: ImageMagickService,
    pub downloader: DownloaderService,
}
