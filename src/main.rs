mod config;
mod helpers;
mod models;
mod routes;
mod services;
mod state;

use actix_web::{middleware, web, App, HttpResponse, HttpServer};
use anyhow::{Context, Result};
use sqlx::postgres::PgConnectOptions;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

use config::AppConfig;
use models::image::ImageBackend;
use models::image_db::DbBackend;
use models::image_file::FileBackend;
use services::cloudfront::CloudfrontService;
use services::downloader::DownloaderService;
use services::imagemagick::ImageMagickService;
use services::s3::S3Service;
use state::AppState;

/// Makes a raw HTTP/1.1 GET to /healthcheck using only stdlib TCP.
/// Returns true on HTTP 200, false on any error or non-200 response.
/// Invoked when the binary is called as: server healthcheck [hostname [port]]
fn run_healthcheck(host: &str, port: u16) -> bool {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::Duration;

    let Ok(mut stream) = TcpStream::connect(format!("{host}:{port}")) else {
        return false;
    };
    stream.set_read_timeout(Some(Duration::from_secs(4))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(4))).ok();

    let req = "GET /healthcheck HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
    if stream.write_all(req.as_bytes()).is_err() {
        return false;
    }

    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }

    response.starts_with("HTTP/1.1 200")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    if std::env::args().nth(1).as_deref() == Some("healthcheck") {
        let host = std::env::args().nth(2).unwrap_or_else(|| "localhost".to_string());
        let default_port = AppConfig::load().map(|c| c.server.port).unwrap_or(3000);
        let port = std::env::args().nth(3)
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(default_port);
        std::process::exit(if run_healthcheck(&host, port) { 0 } else { 1 });
    }

    if let Err(e) = run().await {
        eprintln!("Fatal error: {:#}", e);
        std::process::exit(1);
    }
    Ok(())
}

async fn run() -> Result<()> {
    // Load config
    let cfg = AppConfig::load().context("Failed to load application config")?;

    // Init tracing
    let filter = EnvFilter::try_new(&cfg.log.level)
        .unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();

    info!("image-service starting on port {}", cfg.server.port);

    // Build backend (and optionally run DB migrations)
    let backend: Arc<dyn ImageBackend> = match cfg.image_metadata.storage.trim() {
        "db" => {
            info!("Using PostgreSQL backend, running migrations…");
            let pool = build_pg_pool(&cfg).await?;
            run_migrations(&pool).await?;
            Arc::new(DbBackend::new(pool))
        }
        _ => {
            info!("Using file backend at {}", cfg.image_metadata.parent_path);
            Arc::new(FileBackend::new(cfg.image_metadata.parent_path.clone()))
        }
    };

    // Build CloudFront service (may fail if key config is malformed)
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

    let server = HttpServer::new(move || {
        App::new()
            .wrap(middleware::Logger::default())
            .app_data(app_state.clone())
            .route(
                "/healthcheck",
                web::get().to(|| async { HttpResponse::Ok().body("OK") }),
            )
            .configure(routes::configure)
            .route("/test", web::get().to(test_page))
            .route("/test/", web::get().to(test_page))
    })
    .bind(("0.0.0.0", port))
    .with_context(|| format!("Failed to bind to port {}", port))?;

    info!("Listening on port {}", port);
    server.run().await.map_err(|e| anyhow::anyhow!(e))
}

async fn build_pg_pool(cfg: &AppConfig) -> Result<PgPool> {
    let db = &cfg.db;
    let mut opts = PgConnectOptions::new()
        .host(&db.host)
        .port(db.port)
        .database(&db.database)
        .username(&db.username)
        .password(&db.password);

    if db.ssl.enabled {
        use sqlx::postgres::PgSslMode;
        opts = opts.ssl_mode(if db.ssl.verify {
            PgSslMode::VerifyFull
        } else {
            PgSslMode::Require
        });
        if !db.ssl.ca_cert_file.is_empty() {
            opts = opts.ssl_root_cert(&db.ssl.ca_cert_file);
        }
    }

    sqlx::pool::PoolOptions::new()
        .min_connections(db.minpool)
        .max_connections(db.maxpool)
        .connect_with(opts)
        .await
        .context("Failed to connect to PostgreSQL")
}

async fn test_page() -> HttpResponse {
    static HTML: &str = include_str!("../static/index.html");
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(HTML)
}

async fn run_migrations(pool: &PgPool) -> Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("Failed to run database migrations")?;
    info!("Database migrations completed");
    Ok(())
}
