use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;
use image::{DynamicImage, ImageFormat};
use regex::Regex;
use sha1::{Digest, Sha1};
use tokio::sync::Semaphore;
use crate::log::LogSink;
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

            // Decode HTML entities in the src value before URL parsing.
            // Sanitized XHTML uses `&amp;` inside attribute values (correct for XHTML),
            // but the HTTP client needs the actual `&` in query strings. Without this,
            // requests go out with literal `&amp;` and servers return 404s or garbage
            // that fails to decode as an image.
            let decoded = decode_html_entities(&raw);

            // Resolve to absolute URL
            let abs = if let Ok(parsed) = url::Url::parse(&decoded) {
                // Already absolute — only allow http/https
                if parsed.scheme() != "http" && parsed.scheme() != "https" {
                    return None;
                }
                parsed.to_string()
            } else {
                // Try to resolve as relative
                match base.join(&decoded) {
                    Ok(resolved) => {
                        if resolved.scheme() != "http" && resolved.scheme() != "https" {
                            return None;
                        }
                        resolved.to_string()
                    }
                    Err(_) => return None,
                }
            };
            // Return (original raw src, decoded absolute URL).
            // raw is the map key that matches what's in the HTML.
            // abs is what we actually fetch, and what url_sha1 is computed on.
            Some((raw, abs))
        })
        .collect()
}

/// Decodes the small set of HTML entities that appear in XHTML attribute values.
/// Ammonia's output only ever produces these five, so handling them is sufficient.
fn decode_html_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string(); // fast path: nothing to decode
    }
    s.replace("&amp;", "&")
     .replace("&lt;", "<")
     .replace("&gt;", ">")
     .replace("&quot;", "\"")
     .replace("&#39;", "'")
     .replace("&apos;", "'")
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
    static HAS_SRC_RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();

    let tag_re = IMG_TAG_RE.get_or_init(|| {
        // Matches complete self-closing <img ... /> tags (XHTML output from fixup_xhtml)
        Regex::new(r#"(?i)<img\b[^>]*/>"#).unwrap()
    });
    
    let ext_src_re = EXT_SRC_RE.get_or_init(|| {
        Regex::new(r#"(?i)\bsrc="https?://"#).unwrap()
    });

    let has_src_re = HAS_SRC_RE.get_or_init(|| {
        Regex::new(r#"(?i)\bsrc="#).unwrap()
    });

    tag_re.replace_all(html, |caps: &regex::Captures| {
        let tag = &caps[0];
        
        // 1. Drop the tag if it has an external URL that wasn't embedded
        if ext_src_re.is_match(tag) {
            return String::new();
        }
        
        // 2. Drop the tag if it is completely missing a src attribute
        if !has_src_re.is_match(tag) {
            return String::new();
        }

        // 3. Otherwise, it's a valid local image — keep it
        tag.to_string()
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
/// Errors are routed through `log.println()`.
/// without corrupting the cursor-tracking used for in-place updates.
pub async fn download_image(
    client:    &wreq::Client,
    raw_src:   String,
    abs_url:   String,
    max_width: u32,
    times:     &HostTimes,
    sem:       &Arc<Semaphore>,
    log:       LogSink,
) -> Option<ProcessedImage> {
    // Hold the permit for the full download + process cycle.
    let _permit = sem.acquire().await.unwrap();

    let bytes = match crate::throttle::throttled_get(client, &abs_url, times).await {
        Err(e) => {
            log.println(&format!("Image fetch error ({}): {}", abs_url, e));
            let _ = raw_src;
            return None;
        }
        Ok(resp) => match resp.bytes().await {
            Err(e) => {
                log.println(&format!("Image read error ({}): {}", abs_url, e));
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
        process_image_bytes(bytes.to_vec(), &abs_url_clone, &sha1, max_width, &log)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn url_sha1_deterministic() {
        let h1 = url_sha1("https://example.com/image.jpg");
        let h2 = url_sha1("https://example.com/image.jpg");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 40); // SHA1 hex is always 40 chars
    }

    #[test]
    fn url_sha1_different_urls_produce_different_hashes() {
        let h1 = url_sha1("https://example.com/a.jpg");
        let h2 = url_sha1("https://example.com/b.jpg");
        assert_ne!(h1, h2);
    }

    #[test]
    fn decode_html_entities_amp() {
        assert_eq!(decode_html_entities("a&amp;b"), "a&b");
    }

    #[test]
    fn decode_html_entities_lt_gt() {
        assert_eq!(decode_html_entities("&lt;div&gt;"), "<div>");
    }

    #[test]
    fn decode_html_entities_quotes() {
        assert_eq!(decode_html_entities("&quot;"), "\"");
        assert_eq!(decode_html_entities("&apos;"), "'");
        assert_eq!(decode_html_entities("&#39;"), "'");
    }

    #[test]
    fn decode_html_entities_mixed() {
        let input = "url?a=1&amp;b=2&amp;c=3";
        assert_eq!(decode_html_entities(input), "url?a=1&b=2&c=3");
    }

    #[test]
    fn decode_html_entities_fastpath_no_ampersand() {
        let s = "no entities here";
        assert_eq!(decode_html_entities(s), s);
    }

    #[test]
    fn extract_image_urls_absolute() {
        let html = r#"<img src="https://cdn.example.com/photo.jpg" />"#;
        let pairs = extract_image_urls(html, "https://example.com/article");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, "https://cdn.example.com/photo.jpg");
        assert_eq!(pairs[0].1, "https://cdn.example.com/photo.jpg");
    }

    #[test]
    fn extract_image_urls_relative_resolved() {
        let html = r#"<img src="../images/photo.jpg" />"#;
        let pairs = extract_image_urls(html, "https://example.com/posts/article");
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].1, "https://example.com/images/photo.jpg");
    }

    #[test]
    fn extract_image_urls_entity_encoded_src_decoded() {
        let html = r#"<img src="https://cdn.example.com/image?a=1&amp;b=2" />"#;
        let pairs = extract_image_urls(html, "https://example.com/article");
        assert_eq!(pairs.len(), 1);
        // raw src preserved as-is, but abs URL has decoded ampersand
        assert_eq!(pairs[0].0, "https://cdn.example.com/image?a=1&amp;b=2");
        assert_eq!(pairs[0].1, "https://cdn.example.com/image?a=1&b=2");
    }

    #[test]
    fn extract_image_urls_skips_data_uri() {
        let html = r#"<img src="data:image/png;base64,abc" />"#;
        let pairs = extract_image_urls(html, "https://example.com/article");
        assert!(pairs.is_empty());
    }

    #[test]
    fn extract_image_urls_skips_non_http_scheme() {
        let html = r#"<img src="ftp://example.com/img.jpg" />"#;
        let pairs = extract_image_urls(html, "https://example.com/article");
        assert!(pairs.is_empty());
    }

    #[test]
    fn extract_image_urls_invalid_article_url_returns_empty() {
        let html = r#"<img src="photo.jpg" />"#;
        let pairs = extract_image_urls(html, "not-a-url");
        assert!(pairs.is_empty());
    }

    #[test]
    fn rewrite_img_srcs_replaces_matching_src() {
        let html = r#"<img src="https://cdn.example.com/photo.jpg" />"#;
        let mut map = HashMap::new();
        map.insert("https://cdn.example.com/photo.jpg".to_string(), "images/abc123.jpeg".to_string());
        let result = rewrite_img_srcs(html, &map);
        assert!(result.contains(r#"src="images/abc123.jpeg""#));
    }

    #[test]
    fn rewrite_img_srcs_no_match_unchanged() {
        let html = r#"<img src="https://cdn.example.com/other.jpg" />"#;
        let mut map = HashMap::new();
        map.insert("https://cdn.example.com/photo.jpg".to_string(), "images/abc123.jpeg".to_string());
        let result = rewrite_img_srcs(html, &map);
        assert!(result.contains(r#"src="https://cdn.example.com/other.jpg""#));
    }

    #[test]
    fn rewrite_img_srcs_empty_map_noop() {
        let html = r#"<img src="https://cdn.example.com/photo.jpg" />"#;
        let result = rewrite_img_srcs(html, &HashMap::new());
        assert_eq!(result, html);
    }

    #[test]
    fn strip_external_imgs_removes_http_src() {
        let html = r#"<p>text</p><img src="https://cdn.example.com/photo.jpg" />after"#;
        let result = strip_external_imgs(html);
        assert!(!result.contains("<img"));
        assert!(result.contains("<p>text</p>"));
    }

    #[test]
    fn strip_external_imgs_keeps_local_src() {
        let html = r#"<img src="images/abc123.jpeg" />"#;
        let result = strip_external_imgs(html);
        assert!(result.contains("<img"));
        assert!(result.contains("images/abc123.jpeg"));
    }

    #[test]
    fn sanitize_data_attrs_blanks_json_value() {
        let html = r#"<img data-attrs="{&quot;src&quot;:&quot;https://cdn.example.com/img.jpg&quot;}" src="photo.jpg" />"#;
        let result = sanitize_data_attrs(html);
        assert!(result.contains(r#"data-attrs="""#));
        assert!(result.contains(r#"src="photo.jpg""#)); // src untouched
    }

    #[test]
    fn sanitize_data_attrs_blanks_image_extension() {
        let html = r#"<img data-src="https://example.com/photo.png" src="fallback.jpg" />"#;
        let result = sanitize_data_attrs(html);
        assert!(result.contains(r#"data-src="""#));
    }

    #[test]
    fn sanitize_data_attrs_keeps_innocuous_value() {
        let html = r#"<div data-id="12345" data-count="42">content</div>"#;
        let result = sanitize_data_attrs(html);
        assert!(result.contains(r#"data-id="12345""#));
        assert!(result.contains(r#"data-count="42""#));
    }
}

fn process_image_bytes(
    bytes: Vec<u8>,
    abs_url: &str,
    sha1: &str,
    max_width: u32,
    log: &LogSink,
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

    // Fast path: read just the header to learn format + dimensions.
    // For JPEG/PNG already within max_width, ship the original bytes verbatim
    // — no decode, no resize, no re-encode.
    if let Ok(reader) = image::ImageReader::new(Cursor::new(&bytes)).with_guessed_format() {
        let ext_opt = match reader.format() {
            Some(image::ImageFormat::Jpeg) => Some("jpeg"),
            Some(image::ImageFormat::Png)  => Some("png"),
            _ => None,
        };
        if let Some(ext) = ext_opt {
            if let Ok((w, _h)) = reader.into_dimensions() {
                if w <= max_width {
                    return Some(ProcessedImage {
                        url_sha1:     sha1.to_string(),
                        original_url: abs_url.to_string(),
                        filename:     format!("images/{}.{}", sha1, ext),
                        data:         bytes,
                    });
                }
            }
        }
    }

    // Slow path: full decode (the image is too wide, or it's webp/gif/ico/etc.)
    let img = match image::load_from_memory(&bytes) {
        Ok(img) => img,
        Err(e) => {
            log.println(&format!("Image decode error ({}): {}", abs_url, e));
            return None;
        }
    };

    let has_alpha = img.color().has_alpha();

    let img: DynamicImage = if has_alpha {
        img.into_rgba8().into()
    } else {
        img.into_rgb8().into()
    };

    // Resize if wider than max_width
    let img = if img.width() > max_width {
        let new_w = max_width;
        let new_h = ((img.height() as f64 * max_width as f64 / img.width() as f64) as u32).max(1);
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Triangle)
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
        log.println(&format!("Image encode error ({}): {}", abs_url, e));
        return None;
    }

    Some(ProcessedImage {
        url_sha1:     sha1.to_string(),
        original_url: abs_url.to_string(),
        filename,
        data: buf,
    })
}
