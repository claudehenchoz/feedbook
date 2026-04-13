use std::io::Cursor;
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use chrono::{DateTime, Datelike, Timelike, Utc};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use fast_image_resize::{FilterType, ResizeAlg, ResizeOptions, Resizer};
use crate::error::AppError;

static FONT_BYTES: &[u8] = include_bytes!("../fonts/texgyreheros-bold.otf");

const W: u32 = 1262;
const H: u32 = 1680;
const COLS: u32 = 9;
const ROWS: u32 = 10;

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

/// Discovers the site's icons via site_icons (HTML scraping + manifest + favicon.ico),
/// picks the smallest raster icon that is still ≥ 64 px wide/tall (or the largest
/// available if none meet the threshold), then fetches its raw bytes.
pub async fn fetch_favicon(client: &reqwest::Client, feed_url: &str) -> Option<Vec<u8>> {
    let parsed = url::Url::parse(feed_url).ok()?;
    let base_url = format!("{}://{}", parsed.scheme(), parsed.host_str()?);

    let icons = site_icons::SiteIcons::new()
        .load_website(&base_url, false)
        .await
        .ok()?;

    // Filter to raster formats only (SVG can't be decoded with the image crate)
    let raster: Vec<_> = icons
        .iter()
        .filter(|i| !matches!(i.info, site_icons::IconInfo::SVG { .. }))
        .collect();

    if raster.is_empty() {
        return None;
    }

    // Icons are sorted largest-first. Walk from the end to find the smallest
    // icon that still meets the 64 px threshold; fall back to the largest.
    let best = raster
        .iter()
        .rev()
        .find(|i| i.info.size().is_some_and(|s| s.max_rect() >= 64))
        .or_else(|| raster.first())
        .copied()?;

    let bytes = client
        .get(best.url.as_str())
        .send()
        .await
        .ok()?
        .bytes()
        .await
        .ok()?;

    if bytes.is_empty() {
        return None;
    }
    Some(bytes.to_vec())
}

/// Generates the cover image and returns it as PNG bytes.
pub fn generate_cover(
    title: &str,
    date: Option<DateTime<Utc>>,
    favicon_data: Option<&[u8]>,
) -> Result<Vec<u8>, AppError> {
    // White canvas
    let mut img = RgbaImage::from_pixel(W, H, Rgba([255, 255, 255, 255]));

    let font = FontRef::try_from_slice(FONT_BYTES)
        .map_err(|e| AppError::Other(format!("Font load error: {e}")))?;

    // --- Title (top-left, adaptive font size) ---
    let target_width = W as f32 * 0.85;
    let mut font_size: f32 = 140.0;
    while font_size > 40.0 {
        if measure_text_width(title, PxScale::from(font_size), &font) <= target_width {
            break;
        }
        font_size -= 2.0;
    }
    let title_scale = PxScale::from(font_size);
    let ascent = font.as_scaled(title_scale).ascent();
    let descent = font.as_scaled(title_scale).descent(); // negative
    let title_baseline = 40.0 + ascent;
    draw_text(&mut img, title, 20.0, title_baseline, title_scale, Rgba([0, 0, 0, 255]), &font);

    // --- Date block (top-right, 3 lines, right-aligned) ---
    if let Some(date) = date {
        let date_scale = PxScale::from(20.0);
        let date_ascent = font.as_scaled(date_scale).ascent();
        let line_spacing: f32 = 22.0;
        let right_margin: f32 = 20.0;
        let date_top: f32 = 25.0;

        let weekday = date.format("%A").to_string().to_lowercase();
        let day = date.day();
        let month = date.format("%B").to_string().to_lowercase();
        let year = date.year();
        let day_month_year = format!("{day} {month} {year}");
        let time_str = format!("{:02}:{:02}", date.hour(), date.minute());

        for (i, line) in [weekday.as_str(), day_month_year.as_str(), time_str.as_str()].iter().enumerate() {
            let line_w = measure_text_width(line, date_scale, &font);
            let x = W as f32 - right_margin - line_w;
            let y = date_top + date_ascent + i as f32 * line_spacing;
            draw_text(&mut img, line, x, y, date_scale, Rgba([0, 0, 0, 255]), &font);
        }
    }

    // --- Favicon pattern (9×10 tiled grid, rotated 10° CCW, 50% opacity) ---
    let cell_w = W / COLS;
    let pattern_start_y = (title_baseline - descent + 20.0) as u32;
    let cell_h = (H.saturating_sub(pattern_start_y)) / ROWS;

    if cell_w > 0 && cell_h > 0 {
        if let Some(fav_bytes) = favicon_data {
            if let Some(fav_rgba) = decode_and_resize_favicon(fav_bytes, cell_w, cell_h) {
                draw_favicon_pattern(&mut img, &fav_rgba, cell_w, cell_h, pattern_start_y);
            }
        }
    }

    // Encode to PNG
    let mut buf: Vec<u8> = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| AppError::Other(format!("PNG encode error: {e}")))?;
    Ok(buf)
}

fn decode_and_resize_favicon(bytes: &[u8], w: u32, h: u32) -> Option<RgbaImage> {
    let src = image::load_from_memory(bytes).ok()?.into_rgba8();
    let src_dyn: DynamicImage = src.into();
    let mut dst = DynamicImage::ImageRgba8(RgbaImage::new(w, h));
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    Resizer::new().resize(&src_dyn, &mut dst, &opts).ok()?;
    Some(dst.into_rgba8())
}

fn draw_favicon_pattern(
    canvas: &mut RgbaImage,
    fav: &RgbaImage,
    cell_w: u32,
    cell_h: u32,
    pattern_start_y: u32,
) {
    let fw = cell_w as f32;
    let fh = cell_h as f32;
    let half_fw = fw / 2.0;
    let half_fh = fh / 2.0;

    // 10° CCW rotation inverse map: to find the source pixel for output (px, py),
    // we rotate the offset vector by 10° CW (the inverse of CCW).
    // For CCW rotation by θ: forward is (x·cosθ - y·sinθ, x·sinθ + y·cosθ)
    // Inverse (CW):          sx = dx·cosθ + dy·sinθ, sy = -dx·sinθ + dy·cosθ
    let angle = 10.0_f32.to_radians();
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    for row in 0..ROWS {
        for col in 0..COLS {
            let cell_x = col * cell_w;
            let cell_y = pattern_start_y + row * cell_h;

            for py in 0..cell_h {
                for px in 0..cell_w {
                    let dx = px as f32 - half_fw;
                    let dy = py as f32 - half_fh;

                    let sx = dx * cos_a + dy * sin_a + half_fw;
                    let sy = -dx * sin_a + dy * cos_a + half_fh;

                    let src_pixel = if sx >= 0.0 && sy >= 0.0
                        && (sx as u32) < cell_w && (sy as u32) < cell_h
                    {
                        *fav.get_pixel(sx as u32, sy as u32)
                    } else {
                        Rgba([255, 255, 255, 255])
                    };

                    let canvas_x = cell_x + px;
                    let canvas_y = cell_y + py;
                    if canvas_x >= W || canvas_y >= H {
                        continue;
                    }

                    // Composite src_pixel at 50% opacity over the existing canvas pixel
                    let bg = *canvas.get_pixel(canvas_x, canvas_y);
                    let src_a = src_pixel[3] as f32 / 255.0 * 0.5;
                    let blended = Rgba([
                        blend(src_pixel[0], bg[0], src_a),
                        blend(src_pixel[1], bg[1], src_a),
                        blend(src_pixel[2], bg[2], src_a),
                        255,
                    ]);
                    canvas.put_pixel(canvas_x, canvas_y, blended);
                }
            }
        }
    }
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
