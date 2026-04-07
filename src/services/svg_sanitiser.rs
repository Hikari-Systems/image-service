use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use tempfile::Builder as TempBuilder;
use tokio::fs;

/// Sanitise an SVG file using `ammonia`, writing the clean output to a new temp file.
/// Returns the path of the sanitised file; caller is responsible for deletion.
pub async fn sanitise_svg(input: &Path) -> Result<PathBuf> {
    let raw = fs::read_to_string(input)
        .await
        .with_context(|| format!("Failed to read SVG file: {:?}", input))?;

    // ammonia is CPU-bound; run it on the blocking thread pool.
    let clean = tokio::task::spawn_blocking(move || build_clean_svg(&raw))
        .await
        .context("SVG sanitisation task panicked")?
        .context("SVG sanitisation failed")?;

    let tmp = TempBuilder::new()
        .suffix(".svg")
        .tempfile()
        .context("Failed to create temp file for sanitised SVG")?;
    let (_, dest_path) = tmp.keep().context("Failed to persist sanitised SVG temp file")?;

    fs::write(&dest_path, clean)
        .await
        .with_context(|| format!("Failed to write sanitised SVG to {:?}", dest_path))?;

    Ok(dest_path)
}

fn build_clean_svg(input: &str) -> Result<String> {
    // Configure ammonia to allow the SVG element set.
    // ammonia is an allowlist-based sanitiser; we extend it to pass SVG tags through.
    let clean = ammonia::Builder::new()
        .add_tags(&[
            "svg", "g", "path", "rect", "circle", "ellipse", "line",
            "polyline", "polygon", "text", "tspan", "defs", "use",
            "symbol", "clipPath", "mask", "filter", "feBlend",
            "feColorMatrix", "feComponentTransfer", "feComposite",
            "feConvolveMatrix", "feDiffuseLighting", "feDisplacementMap",
            "feDistantLight", "feFlood", "feFuncA", "feFuncB", "feFuncG",
            "feFuncR", "feGaussianBlur", "feImage", "feMerge", "feMergeNode",
            "feMorphology", "feOffset", "fePointLight", "feSpecularLighting",
            "feSpotLight", "feTile", "feTurbulence", "linearGradient",
            "radialGradient", "stop", "title", "desc", "metadata",
        ])
        .add_generic_attributes(&[
            "id", "class", "style", "transform", "x", "y", "width", "height",
            "viewBox", "preserveAspectRatio", "xmlns", "xmlns:xlink",
            "d", "fill", "stroke", "stroke-width", "stroke-linecap",
            "stroke-linejoin", "opacity", "fill-opacity", "stroke-opacity",
            "cx", "cy", "r", "rx", "ry", "x1", "y1", "x2", "y2",
            "points", "href", "xlink:href", "gradientUnits",
            "gradientTransform", "offset", "stop-color", "stop-opacity",
            "clip-path", "mask", "filter", "font-size", "font-family",
            "text-anchor", "dominant-baseline",
        ])
        .clean(input)
        .to_string();
    Ok(clean)
}
