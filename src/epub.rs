use std::collections::HashMap;
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

pub fn derive_output_path(feed_title: &str) -> PathBuf {
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
    PathBuf::from(format!("{}.epub", slug))
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

    format!(
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
    )
}

pub fn build_epub(
    feed_title:  &str,
    articles:    &[ScrapedArticle],
    epub_images: &HashMap<String, ProcessedImage>,
    output_path: &PathBuf,
) -> Result<(), AppError> {
    let chapters: Vec<EpubChapter> = articles
        .iter()
        .map(|article| {
            let xhtml = build_chapter_xhtml(article, epub_images);
            let title = article.title.as_deref().unwrap_or("Untitled");
            EpubChapter::new(title).xhtml(xhtml)
        })
        .collect();

    let mut builder = Epub::builder()
        .identifier(feed_title)
        .title(feed_title)
        .language("en")
        .resource(("stylesheet.css", STYLESHEET));

    for img in epub_images.values() {
        builder = builder.resource((img.filename.as_str(), img.data.as_slice()));
    }

    builder
        .chapter(chapters)
        .write()
        .save(output_path)
        .map_err(|e| AppError::Epub(e.to_string()))?;

    Ok(())
}
