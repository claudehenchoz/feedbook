use chrono::{DateTime, Utc};
use dom_smoothie::Readability;
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use crate::error::AppError;
use crate::feed::FeedItem;

#[derive(Clone)]
pub struct ScrapedArticle {
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<DateTime<Utc>>,
    pub html: Option<String>,
}

fn parse_date(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    if let Ok(dt) = DateTime::parse_from_rfc2822(s) {
        return Some(dt.with_timezone(&Utc));
    }
    None
}

pub async fn scrape_articles(
    client: &reqwest::Client,
    items: Vec<FeedItem>,
) -> Vec<ScrapedArticle> {
    if items.is_empty() {
        return Vec::new();
    }

    let client = client.clone();
    let total = items.len() as u64;

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} articles")
            .unwrap()
            .progress_chars("#>-"),
    );

    futures::stream::iter(items)
        .map(|item| {
            let client = client.clone();
            let pb = pb.clone();
            async move {
                let result: Result<ScrapedArticle, AppError> = async {
                    let html = client.get(&item.url).send().await?.text().await?;
                    let url_str = item.url.clone();

                    // Article contains StrTendril (!Send), so convert to owned types inside
                    // the blocking thread before returning across the thread boundary.
                    let (title, byline, published_time, content) =
                        tokio::task::spawn_blocking(move || {
                            Readability::new(html, Some(&url_str), None)
                                .and_then(|mut r| r.parse())
                                .map(|a| (a.title, a.byline, a.published_time, a.content.to_string()))
                        })
                        .await
                        .map_err(|e| AppError::Scraper(e.to_string()))? // JoinError
                        .map_err(|e| AppError::Scraper(e.to_string()))?; // ReadabilityError

                    Ok(ScrapedArticle {
                        url: item.url,
                        title: Some(title).filter(|t| !t.is_empty()).or(item.title),
                        author: byline.or(item.author),
                        date: published_time.as_deref().and_then(parse_date).or(item.date),
                        html: Some(content),
                    })
                }
                .await;
                pb.inc(1);
                result.ok()
            }
        })
        .buffer_unordered(5)
        .filter_map(|r| async move { r })
        .collect::<Vec<_>>()
        .await
}
