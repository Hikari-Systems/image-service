mod config;
mod helpers;
mod models;
mod routes;
mod services;
mod state;

use actix_web::{middleware, web, App, HttpResponse};
use anyhow::{Context, Result};
use std::sync::Arc;
use tracing::info;

use config::AppConfig;
use models::image::ImageBackend;
use models::image_db::DbBackend;
use models::image_file::FileBackend;
use services::cloudfront::CloudfrontService;
use services::downloader::DownloaderService;
use services::imagemagick::ImageMagickService;
use services::s3::S3Service;
use state::AppState;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    hs_utils::healthcheck::check_subcommand(
        AppConfig::load().map(|c| c.server.port).unwrap_or(3000),
    );

    if let Err(e) = run().await {
        eprintln!("Fatal error: {:#}", e);
        std::process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<()> {
    let cfg = AppConfig::load().context("Failed to load application config")?;

    hs_utils::logging::init(&cfg.log.level);

    info!("image-service starting on port {}", cfg.server.port);

    let backend: Arc<dyn ImageBackend> = match cfg.image_metadata.storage.trim() {
        "db" => {
            info!("Using PostgreSQL backend, running migrations…");
            let pool = hs_utils::db::build_pool(&cfg.db).await?;
            run_migrations(&pool).await?;
            Arc::new(DbBackend::new(pool))
        }
        _ => {
            info!("Using file backend at {}", cfg.image_metadata.parent_path);
            Arc::new(FileBackend::new(cfg.image_metadata.parent_path.clone()))
        }
    };

    let cf_service = CloudfrontService::new(&cfg.cloudfront)
        .context("Failed to initialise CloudFront service")?;

    info!("S3 bucket: {}", cfg.s3.bucket_name);

    let app_state = web::Data::new(AppState {
        s3: S3Service::new(&cfg.s3),
        cf: cf_service,
        magick: ImageMagickService::new(&cfg),
        downloader: DownloaderService::new(),
        backend,
        config: cfg.clone(),
    });

    let port = cfg.server.port;

    hs_utils::server::run(port, move || {
        App::new()
            // /healthcheck is polled constantly by the load balancer — keep it
            // out of the request log.
            .wrap(middleware::Logger::default().exclude("/healthcheck"))
            .app_data(app_state.clone())
            .route(
                "/healthcheck",
                web::get().to(|| async { HttpResponse::Ok().body("OK") }),
            )
            .configure(routes::configure)
            .route("/test", web::get().to(test_page))
            .route("/test/", web::get().to(test_page))
    })
    .await
}

async fn test_page() -> HttpResponse {
    static HTML: &str = include_str!("../static/index.html");
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(HTML)
}

async fn run_migrations(pool: &sqlx::PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("Failed to run database migrations")?;
    info!("Database migrations completed");
    Ok(())
}
