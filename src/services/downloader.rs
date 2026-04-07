use anyhow::{Context, Result};
use reqwest::Client;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tracing::debug;

pub struct DownloaderService {
    client: Client,
}

pub struct DownloadedFile {
    pub local_path: PathBuf,
    pub mime_type: String,
}

impl DownloaderService {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .user_agent("image-service/0.1")
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Download `url` to `dest` (which should already include the file extension).
    /// Returns the local path and detected MIME type.
    pub async fn download_image(
        &self,
        url: &str,
        dest: &Path,
    ) -> Result<DownloadedFile> {
        debug!("Downloading {} → {:?}", url, dest);
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("HTTP GET failed for {}", url))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Error downloading image: {} status={}",
                url,
                response.status()
            ));
        }

        let mime_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let mut file = File::create(dest)
            .await
            .with_context(|| format!("Failed to create {:?}", dest))?;

        let bytes = response
            .bytes()
            .await
            .with_context(|| format!("Failed to read response body for {}", url))?;

        file.write_all(&bytes)
            .await
            .with_context(|| format!("Failed to write to {:?}", dest))?;
        file.flush().await?;

        debug!("Downloaded {} ({} bytes, {})", url, bytes.len(), mime_type);

        Ok(DownloadedFile {
            local_path: dest.to_path_buf(),
            mime_type,
        })
    }
}

impl Default for DownloaderService {
    fn default() -> Self {
        Self::new()
    }
}
