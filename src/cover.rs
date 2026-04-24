use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use chrono::{DateTime, Datelike, Timelike, Utc};
use image::{Rgba, RgbaImage};
use crate::error::AppError;

static FONT_BYTES: &[u8] = include_bytes!("../assets/texgyreheros-bold.otf");

const W: u32 = 1262;
const H: u32 = 1680;
const COLS: u32 = 9;
const ROWS: u32 = 11;
const ROTATION_DEGREES: f32 = 5.0;

/// Extracts the second-level domain from a URL as a short display title.
/// "https://www.inoreader.com/..." → "inoreader"
pub fn extract_domain_title(feed_url: &str) -> String {
    let host = match url::Url::parse(feed_url).ok().and_then(|u| u.host_str().map(str::to_owned)) {
        Some(h) => h,
        None => return "feed".to_string(),
    };
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        parts[parts.len() - 2].to_string()
    } else {
        host
    }
}

/// Discovers favicon candidates by scraping `<link rel="icon">` tags from the
/// site's homepage, falling back to `/favicon.ico`. Tries candidates largest-
/// first and returns the first one whose bytes decode as a raster image
/// at least 64 px wide/tall, or the first that decodes at all.
pub async fn fetch_favicon(client: &reqwest::Client, feed_url: &str) -> Option<Vec<u8>> {
    let parsed = url::Url::parse(feed_url).ok()?;
    let scheme = parsed.scheme();
    let original_host = parsed.host_str()?;

    // Attempt 1: use the feed URL's host as-is.
    // For Feedburner-style feeds (feeds.example.com/foo) this often won't have
    // a usable homepage, but for direct feeds (example.com/feed.xml) it will.
    let base1 = url::Url::parse(&format!("{}://{}", scheme, original_host)).ok()?;
    if let Some(bytes) = try_favicon_from_base(client, &base1).await {
        return Some(bytes);
    }

    // Attempt 2: reduce to apex domain and try again.
    // "feeds.arstechnica.com" → "arstechnica.com"
    // "www.bbc.co.uk"         → "bbc.co.uk"
    let apex_host = reduce_to_apex(original_host);
    if apex_host != original_host {
        if let Ok(base2) = url::Url::parse(&format!("{}://{}", scheme, apex_host)) {
            if let Some(bytes) = try_favicon_from_base(client, &base2).await {
                return Some(bytes);
            }
        }
    }

    None
}

/// Tries to find a usable favicon for the site rooted at `base`.
/// Scrapes <link rel="icon"> tags from the homepage, falls back to /favicon.ico,
/// and returns the largest decodable raster image (preferring ≥64 px).
async fn try_favicon_from_base(client: &reqwest::Client, base: &url::Url) -> Option<Vec<u8>> {
    use dom_query::Document;

    let mut candidates: Vec<(u32, url::Url)> = Vec::new();

    // Fetch homepage HTML (best-effort)
    if let Ok(resp) = client.get(base.as_str()).send().await {
        if resp.status().is_success() {
            if let Ok(body) = resp.text().await {
                let doc = Document::from(body.as_str());

                let selectors: &[(&str, u32)] = &[
                    ("link[rel='apple-touch-icon']", 180),
                    ("link[rel='apple-touch-icon-precomposed']", 180),
                    ("link[rel='icon']", 32),
                    ("link[rel='shortcut icon']", 16),
                ];

                for (sel, default_size) in selectors {
                    for node in doc.select(sel).iter() {
                        let href = node.attr("href").unwrap_or_default();
                        if href.is_empty() {
                            continue;
                        }
                        let abs = match base.join(&href) {
                            Ok(u) if u.scheme() == "http" || u.scheme() == "https" => u,
                            _ => continue,
                        };
                        let size = node
                            .attr("sizes")
                            .and_then(|s| s.split(|c: char| c == 'x' || c == 'X').next().map(|n| n.to_string()))
                            .and_then(|n| n.trim().parse::<u32>().ok())
                            .unwrap_or(*default_size);
                        candidates.push((size, abs));
                    }
                }
            }
        }
    }

    // Always try /favicon.ico as a last resort
    if let Ok(u) = base.join("/favicon.ico") {
        candidates.push((16, u));
    }

    if candidates.is_empty() {
        return None;
    }

    // Sort largest-first, dedupe by URL while preserving order
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|(_, u)| seen.insert(u.clone()));

    let mut fallback: Option<Vec<u8>> = None;

    for (_, url) in candidates {
        let bytes = match client.get(url.as_str()).send().await {
            Ok(resp) if resp.status().is_success() => match resp.bytes().await {
                Ok(b) if !b.is_empty() => b.to_vec(),
                _ => continue,
            },
            _ => continue,
        };

        // Quick magic-number check before handing to the decoder — avoids
        // trying to decode HTML error pages served with image content-types.
        if !looks_like_image(&bytes) {
            continue;
        }

        let img = match image::load_from_memory(&bytes) {
            Ok(img) => img,
            Err(_) => continue,
        };

        if img.width().max(img.height()) >= 64 {
            return Some(bytes);
        }

        if fallback.is_none() {
            fallback = Some(bytes);
        }
    }

    fallback
}

/// Reduces a hostname to its registrable domain (apex).
/// "feeds.arstechnica.com" → "arstechnica.com"
/// "www.bbc.co.uk"         → "bbc.co.uk"
/// "example.com"           → "example.com"
#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_domain_title ──────────────────────────────────────────────────

    #[test]
    fn extract_domain_title_www_prefix() {
        assert_eq!(extract_domain_title("https://www.example.com/"), "example");
    }

    #[test]
    fn extract_domain_title_subdomain() {
        assert_eq!(extract_domain_title("https://news.ycombinator.com/"), "ycombinator");
    }

    #[test]
    fn extract_domain_title_bare_domain() {
        assert_eq!(extract_domain_title("https://example.com/some/path"), "example");
    }

    #[test]
    fn extract_domain_title_invalid_url_returns_feed() {
        assert_eq!(extract_domain_title("not-a-url"), "feed");
    }

    // ── reduce_to_apex ────────────────────────────────────────────────────────

    #[test]
    fn reduce_to_apex_strips_www() {
        assert_eq!(reduce_to_apex("www.example.com"), "example.com");
    }

    #[test]
    fn reduce_to_apex_multi_part_tld_co_uk() {
        assert_eq!(reduce_to_apex("www.bbc.co.uk"), "bbc.co.uk");
    }

    #[test]
    fn reduce_to_apex_already_apex() {
        assert_eq!(reduce_to_apex("example.com"), "example.com");
    }

    // ── looks_like_image ──────────────────────────────────────────────────────

    #[test]
    fn looks_like_image_png() {
        let png_magic = [0x89u8, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n', 0, 0, 0, 0];
        assert!(looks_like_image(&png_magic));
    }

    #[test]
    fn looks_like_image_jpeg() {
        let jpeg_magic = [0xFFu8, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0];
        assert!(looks_like_image(&jpeg_magic));
    }

    #[test]
    fn looks_like_image_html_returns_false() {
        let html = b"<!DOCTYPE html><html>";
        assert!(!looks_like_image(html));
    }

    #[test]
    fn looks_like_image_too_short_returns_false() {
        assert!(!looks_like_image(&[0xFF, 0xD8]));
    }

    // ── cover template + date apply ───────────────────────────────────────────

    #[test]
    fn generate_cover_template_returns_valid_png() {
        let result = generate_cover_template("Test Feed", None);
        assert!(result.is_ok(), "generate_cover_template failed: {:?}", result);
        let bytes = result.unwrap();
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
        assert!(bytes.len() > 1000, "cover PNG suspiciously small");
    }

    #[test]
    fn apply_date_to_cover_with_date_succeeds() {
        let template = generate_cover_template("My Feed", None).unwrap();
        let date = chrono::DateTime::parse_from_rfc3339("2024-06-15T08:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let result = apply_date_to_cover(&template, Some(date));
        assert!(result.is_ok());
        let bytes = result.unwrap();
        assert!(bytes.starts_with(&[0x89, b'P', b'N', b'G']), "not a PNG");
    }

    #[test]
    fn apply_date_to_cover_no_date_roundtrips() {
        let template = generate_cover_template("Test Feed", None).unwrap();
        let result = apply_date_to_cover(&template, None);
        assert!(result.is_ok());
    }
}

fn reduce_to_apex(host: &str) -> &str {
    const MULTI_PART_TLDS: &[&str] = &[
        ".co.uk", ".co.jp", ".co.kr", ".co.nz", ".co.za",
        ".com.au", ".com.br", ".com.cn", ".com.mx", ".com.tw",
        ".org.uk", ".net.au", ".ac.uk", ".gov.uk",
    ];

    // Handle multi-part TLDs first
    for suffix in MULTI_PART_TLDS {
        if let Some(prefix) = host.strip_suffix(suffix) {
            // Find the last dot in the prefix to get the registrable part
            return match prefix.rfind('.') {
                Some(idx) => &host[idx + 1..],
                None => host, // host is already "something.co.uk" with no subdomain
            };
        }
    }

    // Standard case: take the last two dot-separated segments
    let dot_count = host.matches('.').count();
    if dot_count <= 1 {
        return host; // already apex (e.g. "example.com") or weird (e.g. "localhost")
    }
    let pos = host.match_indices('.').nth(dot_count - 2).unwrap().0;
    &host[pos + 1..]
}

/// Quick check of file magic to verify bytes look like a supported image format.
/// Avoids handing HTML or other garbage to the image decoder.
fn looks_like_image(bytes: &[u8]) -> bool {
    if bytes.len() < 8 {
        return false;
    }
    // PNG
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']) { return true; }
    // JPEG
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) { return true; }
    // GIF
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") { return true; }
    // ICO (00 00 01 00) or CUR (00 00 02 00)
    if bytes.starts_with(&[0x00, 0x00, 0x01, 0x00]) { return true; }
    // WebP: "RIFF" + 4 size bytes + "WEBP"
    if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" { return true; }
    // BMP
    if bytes.starts_with(b"BM") { return true; }
    false
}

/// Generates the static cover template (title + favicon grid) without any date/time text.
/// The title is constrained to end before x=800, leaving the top-right corner blank for the date.
/// Store the result in the SQLite cover cache keyed by `"{feed_url}|{name}"`.
pub fn generate_cover_template(
    title: &str,
    favicon_data: Option<&[u8]>,
) -> Result<Vec<u8>, AppError> {
    let mut img = RgbaImage::from_pixel(W, H, Rgba([255, 255, 255, 255]));
    let font = FontRef::try_from_slice(FONT_BYTES)
        .map_err(|e| AppError::Other(format!("Font load error: {e}")))?;

    let margin_x      = 80.0f32;
    let margin_bottom = 80.0f32;

    // Title must end before x=800 so the top-right date zone stays permanently blank.
    let target_width = 800.0 - margin_x; // = 720 px

    let initial_scale   = PxScale::from(140.0);
    let standard_ascent = font.as_scaled(initial_scale).ascent();
    let title_v_center_y = 80.0 + (standard_ascent / 2.0);

    let mut font_size: f32 = 140.0;
    while font_size > 40.0 {
        if measure_text_width(title, PxScale::from(font_size), &font) <= target_width {
            break;
        }
        font_size -= 2.0;
    }
    let title_scale       = PxScale::from(font_size);
    let title_ascent      = font.as_scaled(title_scale).ascent();
    let title_descent_abs = font.as_scaled(title_scale).descent().abs();
    let title_actual_height = title_ascent + title_descent_abs;
    let title_baseline    = title_v_center_y + (title_actual_height / 2.0) - title_descent_abs;

    draw_text(&mut img, title, margin_x, title_baseline, title_scale, Rgba([0, 0, 0, 255]), &font);

    let available_w     = W as f32 - (margin_x * 2.0);
    let cell_size       = (available_w / COLS as f32) as u32;
    let grid_total_h    = (cell_size * ROWS) as f32;
    let title_bottom_y  = title_baseline + title_descent_abs;
    let min_start_y     = title_bottom_y + 60.0;
    let pattern_start_y = (H as f32 - margin_bottom - grid_total_h).max(min_start_y) as u32;

    if cell_size > 0 {
        if let Some(fav_bytes) = favicon_data {
            if let Some(fav_rgba) = decode_and_resize_favicon(fav_bytes, cell_size, cell_size) {
                draw_favicon_pattern(&mut img, &fav_rgba, cell_size, cell_size,
                                     pattern_start_y, margin_x as u32);
            }
        }
    }

    encode_png_fast(&img)
}

/// Decodes a cached cover template PNG and overlays the current date/time in the top-right corner.
/// This is the fast per-run step — no favicon fetch, no grid rendering.
pub fn apply_date_to_cover(
    template_png: &[u8],
    date: Option<DateTime<Utc>>,
) -> Result<Vec<u8>, AppError> {
    let mut img = image::load_from_memory(template_png)
        .map_err(|e| AppError::Other(format!("template PNG decode: {e}")))?
        .into_rgba8();

    if let Some(d) = date {
        let font = FontRef::try_from_slice(FONT_BYTES)
            .map_err(|e| AppError::Other(format!("Font load error: {e}")))?;

        let margin_x         = 80.0f32;
        let date_scale       = PxScale::from(30.0);
        let date_line_height = 36.0f32;

        let weekday        = d.format("%A").to_string().to_lowercase();
        let day_month_year = format!("{} {} {}",
            d.day(), d.format("%B").to_string().to_lowercase(), d.year());
        let time_str       = format!("{:02}:{:02}", d.hour(), d.minute());
        let date_strings   = [weekday, day_month_year, time_str];

        let date_ascent = font.as_scaled(date_scale).ascent();
        for (i, line) in date_strings.iter().enumerate() {
            let line_w = measure_text_width(line, date_scale, &font);
            let x = W as f32 - margin_x - line_w;
            let y = 80.0 + date_ascent + i as f32 * date_line_height;
            draw_text(&mut img, line, x, y, date_scale, Rgba([0, 0, 0, 255]), &font);
        }
    }

    encode_png_fast(&img)
}


// Simplified PNG encoding using the `png` crate, which is faster than `image`'s encoder.
fn encode_png_fast(img: &RgbaImage) -> Result<Vec<u8>, AppError> {
    let mut buf = Vec::new();
    let mut encoder = png::Encoder::new(&mut buf, img.width(), img.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header()
        .map_err(|e| AppError::Other(format!("png header: {e}")))?;
    writer.write_image_data(img.as_raw())
        .map_err(|e| AppError::Other(format!("png data: {e}")))?;
    drop(writer);
    Ok(buf)
}

fn decode_and_resize_favicon(bytes: &[u8], cell_w: u32, cell_h: u32) -> Option<RgbaImage> {
    let src = image::load_from_memory(bytes).ok()?.into_rgba8();

    // Force a square dimension (90% of the smaller cell dimension)
    // so icons don't touch each other when rotated.
    let side = (cell_w.min(cell_h) as f32 * 0.9) as u32;

    let resized = image::imageops::resize(
        &src,
        side,
        side,
        image::imageops::FilterType::Lanczos3,
    );
    Some(resized)
}

fn draw_favicon_pattern(
    canvas: &mut RgbaImage,
    fav: &RgbaImage,
    cell_w: u32,
    cell_h: u32,
    pattern_start_y: u32,
    margin_x: u32,
) {
    // Step 1: Render one stamp with all the expensive work (rotation,
    // supersampling, bilinear filtering, opacity).
    let stamp = render_stamp(fav, cell_w, cell_h);

    // Step 2: Blit that stamp onto every grid cell — just alpha compositing.
    for row in 0..ROWS {
        for col in 0..COLS {
            let cell_x = margin_x + col * cell_w;
            let cell_y = pattern_start_y + row * cell_h;

            for py in 0..cell_h {
                for px in 0..cell_w {
                    let (pm_r, pm_g, pm_b, final_a) = stamp[(py * cell_w + px) as usize];

                    if final_a > 0.0 {
                        let canvas_x = cell_x + px;
                        let canvas_y = cell_y + py;
                        if canvas_x < W && canvas_y < H {
                            let bg = canvas.get_pixel(canvas_x, canvas_y);
                            let r = (pm_r + bg[0] as f32 * (1.0 - final_a)) as u8;
                            let g = (pm_g + bg[1] as f32 * (1.0 - final_a)) as u8;
                            let b = (pm_b + bg[2] as f32 * (1.0 - final_a)) as u8;
                            canvas.put_pixel(canvas_x, canvas_y, Rgba([r, g, b, 255]));
                        }
                    }
                }
            }
        }
    }
}

/// Renders the rotated, supersampled favicon into a single cell-sized buffer.
/// Each element is (pre_multiplied_r, pre_multiplied_g, pre_multiplied_b, alpha).
fn render_stamp(fav: &RgbaImage, cell_w: u32, cell_h: u32) -> Vec<(f32, f32, f32, f32)> {
    let angle = ROTATION_DEGREES.to_radians();
    let (sin_a, cos_a) = angle.sin_cos();

    let fav_w = fav.width() as f32;
    let fav_h = fav.height() as f32;
    let out_cx = cell_w as f32 / 2.0;
    let out_cy = cell_h as f32 / 2.0;
    let in_cx = fav_w / 2.0;
    let in_cy = fav_h / 2.0;

    let ss_offsets = [(-0.25f32, -0.25f32), (0.25, 0.25)]; // 2x supersampling (was 4x)
    let opacity = 0.5;

    let mut stamp = vec![(0.0f32, 0.0f32, 0.0f32, 0.0f32); (cell_w * cell_h) as usize];

    for py in 0..cell_h {
        for px in 0..cell_w {
            let mut r_acc = 0.0f32;
            let mut g_acc = 0.0f32;
            let mut b_acc = 0.0f32;
            let mut a_acc = 0.0f32;

            for (ox, oy) in ss_offsets {
                let dx = (px as f32 + ox) - out_cx;
                let dy = (py as f32 + oy) - out_cy;
                let sx = dx * cos_a + dy * sin_a + in_cx;
                let sy = -dx * sin_a + dy * cos_a + in_cy;

                if sx >= 0.0 && sy >= 0.0 && sx < fav_w - 1.0 && sy < fav_h - 1.0 {
                    let p = get_pixel_bilinear(fav, sx, sy);
                    let sample_a = (p[3] as f32 / 255.0) * opacity;
                    r_acc += p[0] as f32 * sample_a;
                    g_acc += p[1] as f32 * sample_a;
                    b_acc += p[2] as f32 * sample_a;
                    a_acc += sample_a;
                }
            }

            let final_a = a_acc / 2.0;
            stamp[(py * cell_w + px) as usize] = (r_acc / 2.0, g_acc / 2.0, b_acc / 2.0, final_a);
        }
    }

    stamp
}

fn get_pixel_bilinear(img: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let w = img.width() as usize;
    let h = img.height() as usize;
    let raw = img.as_raw();

    let x1 = (x.floor() as usize).min(w - 1);
    let y1 = (y.floor() as usize).min(h - 1);
    let x2 = (x1 + 1).min(w - 1);
    let y2 = (y1 + 1).min(h - 1);

    let fx = x - x.floor();
    let fy = y - y.floor();

    let p = |xi: usize, yi: usize| &raw[yi * w * 4 + xi * 4..][..4];
    let (p11, p21, p12, p22) = (p(x1, y1), p(x2, y1), p(x1, y2), p(x2, y2));

    let mut res = [0u8; 4];
    for i in 0..4 {
        let top    = p11[i] as f32 * (1.0 - fx) + p21[i] as f32 * fx;
        let bottom = p12[i] as f32 * (1.0 - fx) + p22[i] as f32 * fx;
        res[i] = (top * (1.0 - fy) + bottom * fy).round() as u8;
    }
    Rgba(res)
}

#[inline]
fn blend(fg: u8, bg: u8, alpha: f32) -> u8 {
    (fg as f32 * alpha + bg as f32 * (1.0 - alpha)) as u8
}

/// Sums the horizontal advance of every glyph in `text` at the given scale.
fn measure_text_width(text: &str, scale: PxScale, font: &FontRef<'_>) -> f32 {
    let scaled = font.as_scaled(scale);
    text.chars().map(|c| scaled.h_advance(font.glyph_id(c))).sum()
}

/// Rasterizes `text` onto `img` starting at (`x`, `y`) (baseline origin).
fn draw_text(
    img: &mut RgbaImage,
    text: &str,
    x: f32,
    y: f32,
    scale: PxScale,
    color: Rgba<u8>,
    font: &FontRef<'_>,
) {
    let scaled = font.as_scaled(scale);
    let mut cursor_x = x;
    for c in text.chars() {
        let glyph_id = font.glyph_id(c);
        let glyph = glyph_id.with_scale_and_position(scale, ab_glyph::point(cursor_x, y));
        cursor_x += scaled.h_advance(glyph_id);
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bb = outlined.px_bounds();
            outlined.draw(|rx, ry, cov| {
                let px = bb.min.x as i32 + rx as i32;
                let py = bb.min.y as i32 + ry as i32;
                if px < 0 || py < 0 || px >= W as i32 || py >= H as i32 {
                    return;
                }
                let existing = *img.get_pixel(px as u32, py as u32);
                img.put_pixel(px as u32, py as u32, Rgba([
                    blend(color[0], existing[0], cov),
                    blend(color[1], existing[1], cov),
                    blend(color[2], existing[2], cov),
                    255,
                ]));
            });
        }
    }
}
