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

/// `argv[1] == "transcode-sweep"` → `Some(batch override)`, where `argv[2]` is an
/// optional image count. Returns `None` for a normal server start.
///
/// Parsed rather than handed to a CLI crate because this is the second subcommand
/// in the binary and the first (`healthcheck`) is argv-matched too — a dependency
/// would be more machinery than the feature.
fn transcode_sweep_subcommand() -> Option<Option<u32>> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("transcode-sweep") {
        return None;
    }
    Some(args.next().and_then(|n| n.parse().ok()))
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

    // `server transcode-sweep [n]` — run one pass and exit, instead of serving.
    //
    // The same shape as the `healthcheck` subcommand, and for the same reason:
    // it needs the service's own config, credentials and ImageMagick, so a
    // shell script cannot stand in for it. Two things it buys that the in-process
    // loop does not:
    //
    //   * an **external** scheduler can drive the sweep (a systemd timer, a cron
    //     entry, a one-shot container) with `transcodeSweep.enabled` left false —
    //     useful when you want the backlog cleared on a schedule you control
    //     rather than on every replica independently;
    //   * a **manual** trigger, to drain a backlog or check the claim behaves,
    //     without waiting out an interval or restarting anything.
    //
    // It takes the same lease as the loop does, so running it by hand while the
    // loop is also on is safe — the two cannot pick the same image.
    if let Some(n) = transcode_sweep_subcommand() {
        return services::sweeper::run_once(&app_state, n).await;
    }

    // The background pass that completes deferred variants. Gated here rather
    // than inside the loop so the "it will do nothing" case is one line at
    // startup instead of silence: only the Postgres backend can claim an image
    // exclusively, and on the file backend every replica would transcode every
    // image — worse than not sweeping at all.
    if cfg.resize.transcode_sweep.is_enabled() {
        if cfg.image_metadata.storage.trim() == "db" {
            services::sweeper::spawn(app_state.clone().into_inner());
        } else {
            tracing::warn!(
                "transcode sweep is enabled but imageMetadata.storage is {:?}; \
                 only the db backend can claim an image exclusively, so the sweep \
                 is disabled — set storage to \"db\" to use it",
                cfg.image_metadata.storage
            );
        }
    }

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
