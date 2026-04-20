use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use rbook::epub::{Epub, EpubChapter};
use crate::error::AppError;
use crate::images::{self, ProcessedImage};
use crate::scraper::ScrapedArticle;

const STYLESHEET: &str = r#"body {
    font-family: Georgia, 'Times New Roman', serif;
    font-size: 1em;
    line-height: 1.6;
    margin: 1em 2em;
    color: #222;
}
.article-header {
    margin-bottom: 1.5em;
    border-bottom: 1px solid #ccc;
    padding-bottom: 0.8em;
}
h1.article-title {
    font-size: 1.6em;
    margin-bottom: 0.3em;
}
p.article-meta {
    font-size: 0.85em;
    color: #666;
}
.article-body img {
    max-width: 100%;
    height: auto;
}
"#;

pub fn derive_output_path(feed_title: &str, kobo: bool) -> PathBuf {
    let slug: String = feed_title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug = if slug.is_empty() {
        "feed".to_string()
    } else {
        slug
    };
    let ext = if kobo { "kepub.epub" } else { "epub" };
    PathBuf::from(format!("{}.{}", slug, ext))
}

fn inject_kobo_spans(xhtml: &str, chapter: usize) -> String {
    let mut out = String::with_capacity((xhtml.len() as f64 * 1.3) as usize);
    let mut segment: usize = 1;
    let mut in_head = false;
    let mut pos = 0;

    while pos < xhtml.len() {
        // Find next '<'
        let tag_start = match xhtml[pos..].find('<') {
            None => {
                let text = &xhtml[pos..];
                if !in_head && !text.trim().is_empty() {
                    write!(out, r#"<span class="koboSpan" id="kobo.{}.{}">{}</span>"#,
                        chapter, segment, text).unwrap();
                } else {
                    out.push_str(text);
                }
                break;
            }
            Some(rel) => pos + rel,
        };

        // Emit text node before this tag
        let text = &xhtml[pos..tag_start];
        if !in_head && !text.trim().is_empty() {
            write!(out, r#"<span class="koboSpan" id="kobo.{}.{}">{}</span>"#,
                chapter, segment, text).unwrap();
            segment += 1;
        } else {
            out.push_str(text);
        }

        // Find end of tag (quote-aware, handles <!-- and <?)
        let tag_end = find_tag_end(xhtml, tag_start);
        let tag = &xhtml[tag_start..=tag_end];
        out.push_str(tag);

        // Update in_head state based on tag name
        let tag_inner = tag.trim_start_matches('<').trim_end_matches('>').trim();
        // Split on whitespace only (not '/') so that closing tags like
        // "</head>" produce tag_name_lower = "/head", not "".
        let tag_name_lower = tag_inner
            .split(|c: char| c.is_whitespace() || c == '>')
            .next()
            .unwrap_or("")
            .to_lowercase();
        if tag_name_lower == "head" {
            in_head = true;
        } else if tag_name_lower == "/head" {
            in_head = false;
        }

        pos = tag_end + 1;
    }

    out
}

fn find_tag_end(xhtml: &str, tag_start: usize) -> usize {
    let rest = &xhtml[tag_start..];
    // Comments
    if rest.starts_with("<!--") {
        if let Some(rel) = rest[4..].find("-->") {
            return tag_start + 4 + rel + 2; // index of '>' in "-->"
        }
        return xhtml.len() - 1;
    }
    // Processing instructions
    if rest.starts_with("<?") {
        if let Some(rel) = rest[2..].find("?>") {
            return tag_start + 2 + rel + 1; // index of '>' in "?>"
        }
        return xhtml.len() - 1;
    }
    // Normal tag: scan quote-aware
    let bytes = rest.as_bytes();
    let mut i = 1usize;
    let mut in_double = false;
    let mut in_single = false;
    while i < bytes.len() {
        match bytes[i] {
            b'"' if !in_single => in_double = !in_double,
            b'\'' if !in_double => in_single = !in_single,
            b'>' if !in_double && !in_single => return tag_start + i,
            _ => {}
        }
        i += 1;
    }
    xhtml.len() - 1
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn build_chapter_xhtml(
    article: &ScrapedArticle,
    epub_images: &HashMap<String, ProcessedImage>,
    chapter_num: usize,
    kobo: bool,
) -> String {
    let title = article.title.as_deref().unwrap_or("Untitled");
    let author = article.author.as_deref().unwrap_or("Unknown");
    let date = article
        .date
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();

    let html_body = match article.html.as_deref() {
        None | Some("") => String::new(),
        Some(html) => {
            if epub_images.is_empty() {
                html.to_string()
            } else {
                // Build src → local filename map for images present in this article
                let src_to_filename: HashMap<String, String> = images::extract_image_urls(html, &article.url)
                    .into_iter()
                    .filter_map(|(raw_src, abs_url)| {
                        let sha1 = images::url_sha1(&abs_url);
                        epub_images.get(&sha1).map(|img| (raw_src, img.filename.clone()))
                    })
                    .collect();
                let rewritten = images::rewrite_img_srcs(html, &src_to_filename);
                // Drop any <img> tags whose src is still an external URL — these
                // are images that failed to download (including dom_smoothie
                // lazy-image URL mangling). Leaving them causes EPUBCHECK errors.
                images::strip_external_imgs(&rewritten)
            }
        }
    };

    let xhtml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="stylesheet.css"/>
</head>
<body>
  <div class="article-header">
    <h1 class="article-title">{title}</h1>
    <p class="article-meta">By {author} | {date} | <a href="{url}">{url_text}</a></p>
  </div>
  <div class="article-body">
    {html_body}
  </div>
</body>
</html>"#,
        title = escape_html(title),
        author = escape_html(author),
        date = escape_html(&date),
        url = escape_html(&article.url),
        url_text = escape_html(&article.url),
        html_body = html_body,
    );
    if kobo { inject_kobo_spans(&xhtml, chapter_num) } else { xhtml }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_article(url: &str, title: &str) -> ScrapedArticle {
        ScrapedArticle {
            url: url.to_string(),
            title: Some(title.to_string()),
            author: Some("Test Author".to_string()),
            date: Some(chrono::DateTime::parse_from_rfc3339("2024-01-15T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc)),
            html: Some("<p>Article body content.</p>".to_string()),
        }
    }

    // ── derive_output_path ────────────────────────────────────────────────────

    #[test]
    fn derive_output_path_simple() {
        let path = derive_output_path("My Feed", false);
        assert_eq!(path, PathBuf::from("my-feed.epub"));
    }

    #[test]
    fn derive_output_path_kobo() {
        let path = derive_output_path("My Feed", true);
        assert_eq!(path, PathBuf::from("my-feed.kepub.epub"));
    }

    #[test]
    fn derive_output_path_collapses_separators() {
        let path = derive_output_path("Hello -- World", false);
        assert_eq!(path, PathBuf::from("hello-world.epub"));
    }

    #[test]
    fn derive_output_path_empty_title_uses_feed() {
        let path = derive_output_path("", false);
        assert_eq!(path, PathBuf::from("feed.epub"));
    }

    #[test]
    fn derive_output_path_special_chars_stripped() {
        let path = derive_output_path("Tech & Science!", false);
        assert_eq!(path, PathBuf::from("tech-science.epub"));
    }

    // ── escape_html ───────────────────────────────────────────────────────────

    #[test]
    fn escape_html_ampersand() {
        assert_eq!(escape_html("a & b"), "a &amp; b");
    }

    #[test]
    fn escape_html_all_specials() {
        assert_eq!(escape_html(r#"<a href="x">&</a>"#), "&lt;a href=&quot;x&quot;&gt;&amp;&lt;/a&gt;");
    }

    // ── inject_kobo_spans ─────────────────────────────────────────────────────

    #[test]
    fn inject_kobo_spans_wraps_body_text() {
        let xhtml = r#"<html><head><title>T</title></head><body><p>Hello world</p></body></html>"#;
        let result = inject_kobo_spans(xhtml, 1);
        assert!(result.contains("koboSpan"), "got: {}", result);
        assert!(result.contains("kobo.1."), "got: {}", result);
    }

    #[test]
    fn inject_kobo_spans_head_unchanged() {
        let xhtml = r#"<html><head><title>My Title</title></head><body><p>body</p></body></html>"#;
        let result = inject_kobo_spans(xhtml, 1);
        // The title in <head> must NOT be wrapped in koboSpan
        let head_end = result.find("</head>").unwrap();
        let head = &result[..head_end];
        assert!(!head.contains("koboSpan"), "head section has koboSpan: {}", head);
    }

    // ── build_chapter_xhtml ───────────────────────────────────────────────────

    #[test]
    fn build_chapter_xhtml_structure() {
        let article = make_article("https://example.com/article", "Test Article");
        let images = HashMap::new();
        let xhtml = build_chapter_xhtml(&article, &images, 1, false);
        assert!(xhtml.contains("<html"), "missing html tag");
        assert!(xhtml.contains("<head>") || xhtml.contains("<head\n") || xhtml.contains("<head "), "missing head");
        assert!(xhtml.contains("<body>") || xhtml.contains("<body\n"), "missing body");
        assert!(xhtml.contains("article-header"), "missing article-header");
        assert!(xhtml.contains("Test Article"), "missing title");
        assert!(xhtml.contains("Test Author"), "missing author");
    }

    // ── build_epub ────────────────────────────────────────────────────────────

    #[test]
    fn build_epub_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("test.epub");
        let articles = vec![make_article("https://example.com/1", "Article One")];
        let result = build_epub("Test Feed", &articles, &HashMap::new(), None, &output, false);
        assert!(result.is_ok(), "build_epub failed: {:?}", result);
        assert!(output.exists(), "EPUB file was not created");
        assert!(output.metadata().unwrap().len() > 0, "EPUB file is empty");
    }

    #[test]
    fn build_epub_with_kobo_flag() {
        let dir = tempfile::tempdir().unwrap();
        let output = dir.path().join("test.kepub.epub");
        let articles = vec![make_article("https://example.com/1", "Kobo Article")];
        let result = build_epub("Kobo Feed", &articles, &HashMap::new(), None, &output, true);
        assert!(result.is_ok(), "build_epub kobo failed: {:?}", result);
        assert!(output.exists());
    }
}

pub fn build_epub(
    feed_title:  &str,
    articles:    &[ScrapedArticle],
    epub_images: &HashMap<String, ProcessedImage>,
    cover_png:   Option<Vec<u8>>,
    output_path: &PathBuf,
    kobo:        bool,
) -> Result<(), AppError> {
    let article_chapters: Vec<EpubChapter> = articles
        .iter()
        .enumerate()
        .map(|(i, article)| {
            let xhtml = build_chapter_xhtml(article, epub_images, i + 1, kobo);
            let title = article.title.as_deref().unwrap_or("Untitled");
            EpubChapter::new(title).xhtml(xhtml)
        })
        .collect();

    let mut builder = Epub::builder()
        .identifier(feed_title)
        .title(feed_title)
        .creator("Feedbook")
        .language("en")
        .resource(("stylesheet.css", STYLESHEET));

    let mut all_chapters: Vec<EpubChapter> = Vec::new();

    if let Some(png_bytes) = cover_png {
        builder = builder.cover_image(("cover.png", png_bytes));
        let cover_xhtml = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title>Cover</title><style type="text/css">body{margin:0;padding:0;}img{width:100%;display:block;}</style></head>
<body><img src="cover.png" alt="Cover"/></body>
</html>"#;
        all_chapters.push(EpubChapter::new("Cover").xhtml(cover_xhtml));
    }

    all_chapters.extend(article_chapters);

    for img in epub_images.values() {
        builder = builder.resource((img.filename.as_str(), img.data.as_slice()));
    }

    builder
        .chapter(all_chapters)
        .write()
        .save(output_path)
        .map_err(|e| AppError::Epub(e.to_string()))?;

    Ok(())
}
