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
    let mut img = RgbaImage::from_pixel(W, H, Rgba([255, 255, 255, 255]));
    let font = FontRef::try_from_slice(FONT_BYTES)
        .map_err(|e| AppError::Other(format!("Font load error: {e}")))?;

    // --- 1. Define Margins ---
    let margin_x = 80.0; // This creates the "blank space" on the sides
    let margin_bottom = 80.0; // This creates the "blank space" at the bottom

    // --- Title (top-left, aligned to margin) ---
    let target_width = W as f32 - (margin_x * 2.0); // Allow room for margins
    let mut font_size: f32 = 140.0;
    while font_size > 40.0 {
        if measure_text_width(title, PxScale::from(font_size), &font) <= target_width {
            break;
        }
        font_size -= 2.0;
    }
    let title_scale = PxScale::from(font_size);
    let title_baseline = 60.0 + font.as_scaled(title_scale).ascent();
    // Use margin_x here:
    draw_text(&mut img, title, margin_x, title_baseline, title_scale, Rgba([0, 0, 0, 255]), &font);

// --- UPDATED: Date block (top-right, aligned to margin) ---
    if let Some(date) = date {
        // 1. INCREASE FONT SIZE to make it bigger (e.g., 30.0)
        let date_scale = PxScale::from(30.0);
        let date_ascent = font.as_scaled(date_scale).ascent();

        // 2. ADJUST LINE SPACING proportionally to the larger font size
        let date_line_height = 36.0;
        
        let weekday = date.format("%A").to_string().to_lowercase();
        let day_month_year = format!("{} {} {}", date.day(), date.format("%B").to_string().to_lowercase(), date.year());
        let time_str = format!("{:02}:{:02}", date.hour(), date.minute());

        for (i, line) in [weekday.as_str(), day_month_year.as_str(), time_str.as_str()].iter().enumerate() {
            let line_w = measure_text_width(line, date_scale, &font);

            // 3. CONFIRM ALIGNMENT: Right content boundary is W - margin_x. 
            // Text starts at boundary - text width. This remains correct.
            let x = W as f32 - margin_x - line_w;
            let y = 80.0 + date_ascent + i as f32 * date_line_height; // Use new height variable
            draw_text(&mut img, line, x, y, date_scale, Rgba([0, 0, 0, 255]), &font);
        }
    }

    // --- 2. Calculate Tiled Grid with Margins ---
    
    // Available width after margins
    let available_w = W as f32 - (margin_x * 2.0);

    // 1. Force square cells based strictly on the available width
    let cell_size = (available_w / COLS as f32) as u32;

    // 2. Calculate total height of the new perfectly square-celled grid
    let grid_total_h = (cell_size * ROWS) as f32;

    // 3. Shift the pattern down to anchor it to the bottom margin.
    // We use .max() to ensure it doesn't accidentally overlap the title if dimensions change.
    let min_start_y = title_baseline + 60.0; 
    let pattern_start_y = (H as f32 - margin_bottom - grid_total_h).max(min_start_y) as u32;

    if cell_size > 0 {
        if let Some(fav_bytes) = favicon_data {
            // 4. Pass cell_size for both width and height to keep it completely square
            if let Some(fav_rgba) = decode_and_resize_favicon(fav_bytes, cell_size, cell_size) {
                draw_favicon_pattern(&mut img, &fav_rgba, cell_size, cell_size, pattern_start_y, margin_x as u32);
            }
        }
    }

    let mut buf: Vec<u8> = Vec::new();
    img.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png).map_err(|e| AppError::Other(e.to_string()))?;
    Ok(buf)
}

fn decode_and_resize_favicon(bytes: &[u8], cell_w: u32, cell_h: u32) -> Option<RgbaImage> {
    let src = image::load_from_memory(bytes).ok()?.into_rgba8();
    let src_dyn: DynamicImage = src.into();
    
    // 1. Force a square dimension (e.g., 90% of the smaller cell dimension)
    // Adding a small margin (0.9) helps the icons not touch each other when rotated.
    let side = (cell_w.min(cell_h) as f32 * 0.9) as u32; 
    
    // 2. Create the destination as a PERFECT SQUARE
    let mut dst = DynamicImage::ImageRgba8(RgbaImage::new(side, side));
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
    margin_x: u32,
) {
    let angle = ROTATION_DEGREES.to_radians();
    let (sin_a, cos_a) = angle.sin_cos();

    let fav_w = fav.width() as f32;
    let fav_h = fav.height() as f32;
    let out_cx = cell_w as f32 / 2.0;
    let out_cy = cell_h as f32 / 2.0;
    let in_cx = fav_w / 2.0;
    let in_cy = fav_h / 2.0;

    // 2x2 Supersampling offsets within a single pixel
    let ss_offsets = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];
    let opacity = 0.5;

    for row in 0..ROWS {
        for col in 0..COLS {
            let cell_x = margin_x + col * cell_w;
            let cell_y = pattern_start_y + row * cell_h;

            for py in 0..cell_h {
                for px in 0..cell_w {
                    let mut r_acc = 0.0;
                    let mut g_acc = 0.0;
                    let mut b_acc = 0.0;
                    let mut a_acc = 0.0;

                    // Take 4 samples per pixel
                    for (ox, oy) in ss_offsets {
                        let dx = (px as f32 + ox) - out_cx;
                        let dy = (py as f32 + oy) - out_cy;

                        let sx = dx * cos_a + dy * sin_a + in_cx;
                        let sy = -dx * sin_a + dy * cos_a + in_cy;

                        // Check bounds for this specific sample
                        if sx >= 0.0 && sy >= 0.0 && sx < fav_w - 1.0 && sy < fav_h - 1.0 {
                            let p = get_pixel_bilinear(fav, sx, sy);
                            let sample_a = (p[3] as f32 / 255.0) * opacity;
                            
                            // Accumulate pre-multiplied colors
                            r_acc += p[0] as f32 * sample_a;
                            g_acc += p[1] as f32 * sample_a;
                            b_acc += p[2] as f32 * sample_a;
                            a_acc += sample_a;
                        }
                    }

                    // Average the 4 samples
                    let final_a = a_acc / 4.0;
                    if final_a > 0.0 {
                        let canvas_x = cell_x + px;
                        let canvas_y = cell_y + py;

                        if canvas_x < W && canvas_y < H {
                            let bg = canvas.get_pixel(canvas_x, canvas_y);
                            
                            // Blend accumulated foreground over background
                            let r = (r_acc / 4.0 + bg[0] as f32 * (1.0 - final_a)) as u8;
                            let g = (g_acc / 4.0 + bg[1] as f32 * (1.0 - final_a)) as u8;
                            let b = (b_acc / 4.0 + bg[2] as f32 * (1.0 - final_a)) as u8;

                            canvas.put_pixel(canvas_x, canvas_y, Rgba([r, g, b, 255]));
                        }
                    }
                }
            }
        }
    }
}

/// Improved Bilinear sampler with safety checks
fn get_pixel_bilinear(img: &RgbaImage, x: f32, y: f32) -> Rgba<u8> {
    let width = img.width();
    let height = img.height();

    let x1 = (x.floor() as u32).min(width - 1);
    let y1 = (y.floor() as u32).min(height - 1);
    let x2 = (x1 + 1).min(width - 1);
    let y2 = (y1 + 1).min(height - 1);

    let fx = x - x.floor();
    let fy = y - y.floor();

    let p11 = img.get_pixel(x1, y1);
    let p21 = img.get_pixel(x2, y1);
    let p12 = img.get_pixel(x1, y2);
    let p22 = img.get_pixel(x2, y2);

    let mut res = [0u8; 4];
    for i in 0..4 {
        let top = p11[i] as f32 * (1.0 - fx) + p21[i] as f32 * fx;
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
