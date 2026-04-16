use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use image::{DynamicImage, ImageFormat};
use indicatif::ProgressBar;
use regex::Regex;
use sha1::{Digest, Sha1};
use tokio::sync::Semaphore;
use crate::throttle::HostTimes;

pub struct ProcessedImage {
    pub url_sha1:     String,
    pub original_url: String,
    pub filename:     String, // e.g. "images/deadbeef....jpeg"
    pub data:         Vec<u8>,
}

pub fn url_sha1(url: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(url.as_bytes());
    hex::encode(hasher.finalize())
}

/// Extracts `(raw_src_attr, absolute_url)` pairs from sanitized XHTML.
/// Ammonia always double-quotes attributes, so we can rely on `src="..."`.
pub fn extract_image_urls(html: &str, article_url: &str) -> Vec<(String, String)> {
    static IMG_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = IMG_RE.get_or_init(|| {
        Regex::new(r#"(?i)<img\b[^>]*?\bsrc="([^"]*)""#).unwrap()
    });

    let base = match url::Url::parse(article_url) {
        Ok(u) => u,
        Err(_) => return vec![],
    };

    re.captures_iter(html)
        .filter_map(|cap| {
            let raw = cap[1].to_string();
            if raw.is_empty() {
                return None;
            }
            // Skip inline data URIs — nothing to download
            if raw.starts_with("data:") {
                return None;
            }
            // Resolve to absolute URL
            let abs = if let Ok(parsed) = url::Url::parse(&raw) {
                // Already absolute — only allow http/https
                if parsed.scheme() != "http" && parsed.scheme() != "https" {
                    return None;
                }
                raw.clone()
            } else {
                // Try to resolve as relative
                match base.join(&raw) {
                    Ok(resolved) => {
                        if resolved.scheme() != "http" && resolved.scheme() != "https" {
                            return None;
                        }
                        resolved.to_string()
                    }
                    Err(_) => return None,
                }
            };
            Some((raw, abs))
        })
        .collect()
}

/// Rewrites `<img src="raw_src">` references using the provided map.
/// Keys are the raw `src` attribute values captured from HTML.
/// Values are the local EPUB-relative paths (e.g. `"images/sha1hex.jpeg"`).
pub fn rewrite_img_srcs(html: &str, src_to_filename: &HashMap<String, String>) -> String {
    if src_to_filename.is_empty() {
        return html.to_string();
    }

    static IMG_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = IMG_RE.get_or_init(|| {
        Regex::new(r#"(?i)(<img\b[^>]*?\bsrc=")([^"]*)"#).unwrap()
    });

    re.replace_all(html, |caps: &regex::Captures| {
        let prefix = &caps[1]; // `<img ... src="`
        let raw_src = &caps[2];
        if let Some(local) = src_to_filename.get(raw_src) {
            format!("{}{}", prefix, local)
        } else {
            caps[0].to_string()
        }
    })
    .into_owned()
}

/// Removes `<img>` tags whose `src` is still an external http/https URL.
/// Called after `rewrite_img_srcs` to drop any images that were not
/// successfully downloaded — including those whose URLs were mangled
/// by dom_smoothie's lazy-image unwrapping — preventing EPUBCHECK
/// `href-not-in-manifest` errors.
pub fn strip_external_imgs(html: &str) -> String {
    static IMG_TAG_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    static EXT_SRC_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

    let tag_re = IMG_TAG_RE.get_or_init(|| {
        // Matches complete self-closing <img ... /> tags (XHTML output from fixup_xhtml)
        Regex::new(r#"(?i)<img\b[^>]*/>"#).unwrap()
    });
    let src_re = EXT_SRC_RE.get_or_init(|| {
        Regex::new(r#"(?i)\bsrc="https?://"#).unwrap()
    });

    tag_re.replace_all(html, |caps: &regex::Captures| {
        let tag = &caps[0];
        if src_re.is_match(tag) {
            String::new() // drop tag — external URL that wasn't embedded
        } else {
            tag.to_string()
        }
    })
    .into_owned()
}

/// Neutralizes `data-*` attributes whose values could be misinterpreted as image
/// sources by dom_smoothie's `fix_lazy_images` pass. That pass copies any
/// attribute value containing an image-extension substring into `src` or `srcset`,
/// which corrupts `src` when sites put JSON blobs (Substack's `data-attrs`) or
/// bare filenames (Rock Paper Shotgun's `data-uri`) on their img tags.
///
/// We blank the value rather than remove the attribute to keep surrounding HTML
/// well-formed and minimize churn.
pub fn sanitize_data_attrs(html: &str) -> String {
    static ATTR_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    let re = ATTR_RE.get_or_init(|| {
        // Matches: data-foo="value" — any data-* attribute with a double-quoted value.
        Regex::new(r#"(data-[a-zA-Z0-9_:.\-]*)="([^"]*)""#).unwrap()
    });

    // Image extensions dom_smoothie's heuristic keys on.
    const IMG_EXTS: &[&str] = &[
        ".jpg", ".jpeg", ".png", ".gif", ".webp", ".avif", ".bmp", ".svg",
    ];

    re.replace_all(html, |caps: &regex::Captures| {
        let name = &caps[1];
        let value = &caps[2];
        let lower = value.to_ascii_lowercase();

        // Blank if the value contains structural JSON characters (Substack case)
        // OR any image extension substring (Rock Paper Shotgun case, and the
        // general pattern of data-* attrs whose values dom_smoothie would grab).
        let looks_like_image_hint =
            value.contains('{')
            || value.contains("&quot;")
            || IMG_EXTS.iter().any(|ext| lower.contains(ext));

        if looks_like_image_hint {
            format!(r#"{}="""#, name)
        } else {
            caps[0].to_string()
        }
    })
    .into_owned()
}

/// Downloads and processes a single image.
///
/// Acquires one permit from `sem` for the duration of the download and
/// CPU processing, bounding total concurrent image work globally.
/// Uses `throttled_get` to respect per-host rate limits.
///
/// `log_pb` must be a `ProgressBar` belonging to the active `MultiProgress`.
/// Errors are routed through `log_pb.println()` so they appear above the bars
/// without corrupting the cursor-tracking used for in-place updates.
pub async fn download_image(
    client:    &reqwest::Client,
    raw_src:   String,
    abs_url:   String,
    max_width: u32,
    times:     &HostTimes,
    sem:       &Arc<Semaphore>,
    log_pb:    ProgressBar,
) -> Option<ProcessedImage> {
    // Hold the permit for the full download + process cycle.
    let _permit = sem.acquire().await.unwrap();

    let bytes = match crate::throttle::throttled_get(client, &abs_url, times).await {
        Err(e) => {
            log_pb.println(format!("Image fetch error ({}): {}", abs_url, e));
            let _ = raw_src;
            return None;
        }
        Ok(resp) => match resp.bytes().await {
            Err(e) => {
                log_pb.println(format!("Image read error ({}): {}", abs_url, e));
                let _ = raw_src;
                return None;
            }
            Ok(b) => b,
        },
    };

    let sha1 = url_sha1(&abs_url);
    let abs_url_clone = abs_url.clone();
    let _ = raw_src;

    // CPU-intensive work: decode, resize, encode
    tokio::task::spawn_blocking(move || {
        process_image_bytes(bytes.to_vec(), &abs_url_clone, &sha1, max_width, &log_pb)
    })
    .await
    .ok()
    .and_then(|r| r)
}

fn is_svg(bytes: &[u8], url: &str) -> bool {
    let url_lower = url.to_lowercase();
    if url_lower.ends_with(".svg") || url_lower.contains(".svg?") {
        return true;
    }
    // Check first 256 bytes for SVG content markers
    let prefix = &bytes[..bytes.len().min(256)];
    let prefix_str = std::str::from_utf8(prefix).unwrap_or("");
    prefix_str.contains("<svg") || (prefix_str.contains("<?xml") && prefix_str.contains("<svg"))
}

fn process_image_bytes(
    bytes: Vec<u8>,
    abs_url: &str,
    sha1: &str,
    max_width: u32,
    log_pb: &ProgressBar,
) -> Option<ProcessedImage> {
    // Handle SVGs: store as-is without decode/resize
    if is_svg(&bytes, abs_url) {
        let filename = format!("images/{}.svg", sha1);
        return Some(ProcessedImage {
            url_sha1:     sha1.to_string(),
            original_url: abs_url.to_string(),
            filename,
            data: bytes,
        });
    }

    // Decode raster image
    let img = match image::load_from_memory(&bytes) {
        Ok(img) => img,
        Err(e) => {
            log_pb.println(format!("Image decode error ({}): {}", abs_url, e));
            return None;
        }
    };

    // Determine output format based on alpha channel presence
    let has_alpha = img.color().has_alpha();

    // Convert to canonical 8-bit format for resize
    let img: DynamicImage = if has_alpha {
        img.into_rgba8().into()
    } else {
        img.into_rgb8().into()
    };

    // Resize if wider than max_width
    let img = if img.width() > max_width {
        let new_w = max_width;
        let new_h = ((img.height() as f64 * max_width as f64 / img.width() as f64) as u32).max(1);
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };

    // Encode
    let (ext, fmt) = if has_alpha {
        ("png", ImageFormat::Png)
    } else {
        ("jpeg", ImageFormat::Jpeg)
    };
    let filename = format!("images/{}.{}", sha1, ext);

    let mut buf = Vec::new();
    if let Err(e) = img.write_to(&mut Cursor::new(&mut buf), fmt) {
        log_pb.println(format!("Image encode error ({}): {}", abs_url, e));
        return None;
    }

    Some(ProcessedImage {
        url_sha1:     sha1.to_string(),
        original_url: abs_url.to_string(),
        filename,
        data: buf,
    })
}
