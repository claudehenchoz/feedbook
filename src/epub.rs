use std::fs::File;
use std::path::PathBuf;
use epub_builder::{EpubBuilder, EpubContent, ZipLibrary};
use crate::error::AppError;
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

fn build_chapter_xhtml(article: &ScrapedArticle) -> String {
    let title = article.title.as_deref().unwrap_or("Untitled");
    let author = article.author.as_deref().unwrap_or("Unknown");
    let date = article
        .date
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let html_body = article.html.as_deref().unwrap_or("");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml">
<head>
  <title>{title}</title>
  <link rel="stylesheet" type="text/css" href="../stylesheet.css"/>
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
    feed_title: &str,
    articles: &[ScrapedArticle],
    output_path: &PathBuf,
) -> Result<(), AppError> {
    let mut builder = EpubBuilder::new(ZipLibrary::new().map_err(|e| AppError::Epub(e.to_string()))?)
        .map_err(|e| AppError::Epub(e.to_string()))?;

    builder
        .metadata("title", feed_title)
        .map_err(|e| AppError::Epub(e.to_string()))?
        .metadata("lang", "en")
        .map_err(|e| AppError::Epub(e.to_string()))?
        .stylesheet(STYLESHEET.as_bytes())
        .map_err(|e| AppError::Epub(e.to_string()))?;

    for (i, article) in articles.iter().enumerate() {
        let xhtml = build_chapter_xhtml(article);
        let title = article.title.as_deref().unwrap_or("Untitled");
        builder
            .add_content(
                EpubContent::new(format!("article_{i}.xhtml"), xhtml.as_bytes())
                    .title(title),
            )
            .map_err(|e| AppError::Epub(e.to_string()))?;
    }

    let file = File::create(output_path)?;
    builder
        .generate(file)
        .map_err(|e| AppError::Epub(e.to_string()))?;

    Ok(())
}
