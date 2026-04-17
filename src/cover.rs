use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use chrono::{DateTime, Datelike, Timelike, Utc};
use image::{Rgba, RgbaImage};
use crate::error::AppError;

static FONT_BYTES: &[u8] = include_bytes!("../fonts/texgyreheros-bold.otf");

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

/// Generates the cover image and returns it as PNG bytes.
pub fn generate_cover(
    title: &str,
    date: Option<DateTime<Utc>>,
    favicon_data: Option<&[u8]>,
) -> Result<Vec<u8>, AppError> {
    let mut img = RgbaImage::from_pixel(W, H, Rgba([255, 255, 255, 255]));
    let font = FontRef::try_from_slice(FONT_BYTES)
        .map_err(|e| AppError::Other(format!("Font load error: {e}")))?;

    // --- 1. Define Margins ---
    let margin_x = 80.0; // This creates the "blank space" on the sides
    let margin_bottom = 80.0; // This creates the "blank space" at the bottom

    // --- 2. Header Block: Title and Date ---

    // A. Pre-calculate Date Strings and Width
    let date_scale = PxScale::from(30.0);
    let date_line_height = 36.0;
    let mut max_date_w = 0.0;
    
    // We create a temporary scope to manage the lifetime of the string references
    let date_strings = {
        if let Some(d) = date {
            let weekday = d.format("%A").to_string().to_lowercase();
            let day_month_year = format!("{} {} {}", d.day(), d.format("%B").to_string().to_lowercase(), d.year());
            let time_str = format!("{:02}:{:02}", d.hour(), d.minute());
            
            let lines = vec![weekday, day_month_year, time_str];
            for line in &lines {
                let line_w = measure_text_width(line, date_scale, &font);
                if line_w > max_date_w {
                    max_date_w = line_w;
                }
            }
            lines // Return the strings to extend their life
        } else {
            Vec::new()
        }
    };

    // B. Calculate Title Position (locked center)
    let initial_scale = PxScale::from(140.0);
    let standard_ascent = font.as_scaled(initial_scale).ascent();
    // We decide on a constant `title_v_center_y`. We calculate it based on
    // the original code's top spacing of 60px for a standard ascent.
    let title_v_center_y = 80.0 + (standard_ascent / 2.0);

    // C. Calculate Title Font Size to fit, and final baseline
    // Reserve space for margins, the max width of the date block, and a 40px gap
    let date_gap = if max_date_w > 0.0 { 40.0 } else { 0.0 };
    let target_width = W as f32 - (margin_x * 2.0) - max_date_w - date_gap; 
    
    let mut font_size: f32 = 140.0;
    while font_size > 40.0 {
        if measure_text_width(title, PxScale::from(font_size), &font) <= target_width {
            break;
        }
        font_size -= 2.0;
    }
    let title_scale = PxScale::from(font_size);
    let title_ascent = font.as_scaled(title_scale).ascent();
    let title_descent_abs = font.as_scaled(title_scale).descent().abs();
    let title_actual_height = title_ascent + title_descent_abs;
    
    // title_baseline = title_v_center_y + ((ascent + |descent|) / 2) - |descent|
    let title_baseline = title_v_center_y + (title_actual_height / 2.0) - title_descent_abs;
    
    // Draw the title
    draw_text(&mut img, title, margin_x, title_baseline, title_scale, Rgba([0, 0, 0, 255]), &font);

    // D. Draw Date Block (positioning can stay the same as it's not problem)
    if !date_strings.is_empty() {
        let date_ascent = font.as_scaled(date_scale).ascent();
        for (i, line) in date_strings.iter().enumerate() {
            let line_w = measure_text_width(line, date_scale, &font);
            let x = W as f32 - margin_x - line_w;
            let y = 80.0 + date_ascent + i as f32 * date_line_height;
            draw_text(&mut img, line, x, y, date_scale, Rgba([0, 0, 0, 255]), &font);
        }
    }

    // --- 3. Calculate Tiled Grid with Margins ---
    
    // Available width after margins
    let available_w = W as f32 - (margin_x * 2.0);

    // 1. Force square cells based strictly on the available width
    let cell_size = (available_w / COLS as f32) as u32;

    // 2. Calculate total height of the new perfectly square-celled grid
    let grid_total_h = (cell_size * ROWS) as f32;

    // 3. Shift the pattern down to anchor it to the bottom margin.
    // We calculate `min_start_y` based on the actual bottom of the title's
    // dynamic text slot. This ensures consistent spacing.
    let title_bottom_y = title_baseline + font.as_scaled(title_scale).descent().abs();
    let min_start_y = title_bottom_y + 60.0; 
    
    let pattern_start_y = (H as f32 - margin_bottom - grid_total_h).max(min_start_y) as u32;

    if cell_size > 0 {
        if let Some(fav_bytes) = favicon_data {
            // 4. Pass cell_size for both width and height to keep it completely square
            if let Some(fav_rgba) = decode_and_resize_favicon(fav_bytes, cell_size, cell_size) {
                draw_favicon_pattern(&mut img, &fav_rgba, cell_size, cell_size, pattern_start_y, margin_x as u32);
            }
        }
    }

    let buf = encode_png_optimized(&img)?;
    Ok(buf)
}

/// Encodes an RGBA image as a heavily-optimized PNG:
/// 1. Quantize to an indexed palette (imagequant)
/// 2. Write as 8-bit indexed PNG (png crate)
/// 3. Post-process with oxipng for further size reduction
fn encode_png_optimized(img: &RgbaImage) -> Result<Vec<u8>, AppError> {
    let (w, h) = (img.width(), img.height());

    // --- Step 1: Quantize RGBA -> palette ---
    let mut liq = imagequant::new();
    liq.set_quality(0, 90)
        .map_err(|e| AppError::Other(format!("imagequant config: {e}")))?;
    liq.set_speed(4)
        .map_err(|e| AppError::Other(format!("imagequant speed: {e}")))?;

    // imagequant wants &[RGBA] — reinterpret the raw u8 buffer.
    let pixels: &[imagequant::RGBA] = bytemuck::cast_slice(img.as_raw());
    let mut qimg = liq
        .new_image(pixels, w as usize, h as usize, 0.0)
        .map_err(|e| AppError::Other(format!("imagequant new_image: {e}")))?;

    let mut res = liq
        .quantize(&mut qimg)
        .map_err(|e| AppError::Other(format!("imagequant quantize: {e}")))?;
    res.set_dithering_level(1.0)
        .map_err(|e| AppError::Other(format!("imagequant dither: {e}")))?;

    let (palette, indexed_pixels) = res
        .remapped(&mut qimg)
        .map_err(|e| AppError::Other(format!("imagequant remap: {e}")))?;

    // --- Step 2: Write indexed PNG ---
    let mut png_buf: Vec<u8> = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_buf, w, h);
        enc.set_color(png::ColorType::Indexed);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::High);

        let pal_rgb: Vec<u8> = palette.iter().flat_map(|c| [c.r, c.g, c.b]).collect();
        let pal_a: Vec<u8> = palette.iter().map(|c| c.a).collect();
        enc.set_palette(pal_rgb);
        // Only emit a tRNS chunk if any palette entry is non-opaque.
        if pal_a.iter().any(|&a| a < 255) {
            enc.set_trns(pal_a);
        }

        let mut writer = enc
            .write_header()
            .map_err(|e| AppError::Other(format!("png header: {e}")))?;
        writer
            .write_image_data(&indexed_pixels)
            .map_err(|e| AppError::Other(format!("png data: {e}")))?;
    }

    // --- Step 3: Post-process with oxipng ---
    let opts = oxipng::Options::from_preset(2); // 0=fast, 6=max; 2 is ~10x faster for ~3% size penalty
    let optimized = oxipng::optimize_from_memory(&png_buf, &opts)
        .map_err(|e| AppError::Other(format!("oxipng: {e}")))?;

    Ok(optimized)
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
