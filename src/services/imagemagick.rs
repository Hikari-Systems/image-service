use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tempfile::Builder as TempBuilder;
use tokio::process::Command;
use tracing::debug;

use crate::config::AppConfig;

pub struct ImageMagickService {
    bin: String,
    memory_limit: u32,
    map_limit: u32,
}

impl ImageMagickService {
    pub fn new(cfg: &AppConfig) -> Self {
        Self {
            bin: cfg.imagemagick.bin.clone(),
            memory_limit: cfg.resize.memory_limit,
            map_limit: cfg.resize.map_limit,
        }
    }

    /// Resize `source` to `w`x`h` pixels, writing a temp file with `ext` extension.
    /// Pass `w <= 0` and `h <= 0` to skip resizing (copy/convert only).
    /// Returns the path of the output temp file; caller is responsible for deletion.
    pub async fn resize_image(
        &self,
        source: &Path,
        w: i32,
        h: i32,
        extra_opts: &str,
        ext: &str,
    ) -> Result<PathBuf> {
        // Create a named temp file that persists (keep=true) so ImageMagick can write to it.
        let tmp = TempBuilder::new()
            .suffix(ext)
            .tempfile()
            .context("Failed to create temp file for ImageMagick output")?;
        // Keep the file alive on disk; we get back a PathBuf + the file handle drops.
        let (_, dest_path) = tmp.keep().context("Failed to persist temp file")?;

        debug!(
            "resize_image: {}x{} opts={:?} src={:?} dest={:?}",
            w, h, extra_opts, source, dest_path
        );

        let mut args: Vec<String> = vec![
            source.to_string_lossy().into_owned(),
            "-limit".into(),
            "memory".into(),
            format!("{}MiB", self.memory_limit),
            "-limit".into(),
            "map".into(),
            format!("{}MiB", self.map_limit),
        ];

        if !extra_opts.is_empty() {
            for part in extra_opts.split_whitespace() {
                args.push(part.to_string());
            }
        }

        if w > 0 || h > 0 {
            args.push("-resize".into());
            args.push(format!("{}x{}", w, h));
        }

        args.push(dest_path.to_string_lossy().into_owned());

        debug!("ImageMagick command: {} {:?}", self.bin, args);

        let output = Command::new(&self.bin)
            .args(&args)
            .output()
            .await
            .with_context(|| format!("Failed to execute ImageMagick binary: {}", self.bin))?;

        debug!(
            "ImageMagick stdout: {} stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "ImageMagick failed ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        debug!("ImageMagick wrote: {:?}", dest_path);
        Ok(dest_path)
    }
}
