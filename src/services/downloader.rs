use anyhow::{Context, Result};
use hs_utils::http_client::SendTraced as _;
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
    ///
    /// `url` is either a signed CloudFront URL (its query is the credential) or a
    /// caller-supplied source URL, so neither the span nor any log line or error
    /// carries more than [`loggable`] of it: the call is sent opaque (origin only),
    /// and reqwest errors drop their URL, which their `Display` would otherwise print.
    pub async fn download_image(&self, url: &str, dest: &Path) -> Result<DownloadedFile> {
        let shown = loggable(url);
        debug!("Downloading {} → {:?}", shown, dest);
        let response = self
            .client
            .get(url)
            .send_traced_opaque()
            .await
            .map_err(reqwest::Error::without_url)
            .with_context(|| format!("HTTP GET failed for {}", shown))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Error downloading image: {} status={}",
                shown,
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
            .map_err(reqwest::Error::without_url)
            .with_context(|| format!("Failed to read response body for {}", shown))?;

        file.write_all(&bytes)
            .await
            .with_context(|| format!("Failed to write to {:?}", dest))?;
        file.flush().await?;

        debug!(
            "Downloaded {} ({} bytes, {})",
            shown,
            bytes.len(),
            mime_type
        );

        Ok(DownloadedFile {
            local_path: dest.to_path_buf(),
            mime_type,
        })
    }
}

/// `url` without its userinfo, query or fragment: what may be logged of a URL
/// whose query can be a credential (a CloudFront signature, a caller's token).
fn loggable(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut u) => {
            let _ = u.set_username("");
            let _ = u.set_password(None);
            u.set_query(None);
            u.set_fragment(None);
            u.to_string()
        }
        Err(_) => "<unparseable url>".to_string(),
    }
}

impl Default for DownloaderService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::loggable;

    #[test]
    fn a_logged_url_keeps_no_credential() {
        assert_eq!(
            loggable("https://cdn.example.com/a/b.png?Expires=1&Signature=abc&Key-Pair-Id=K"),
            "https://cdn.example.com/a/b.png"
        );
        assert_eq!(
            loggable("https://user:pw@example.com/x.jpg#frag"),
            "https://example.com/x.jpg"
        );
        assert_eq!(loggable("not a url"), "<unparseable url>");
    }
}
