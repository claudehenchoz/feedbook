use std::sync::Arc;
use ammonia::Builder;
use chrono::{DateTime, Utc};
use dom_smoothie::Readability;
use indicatif::ProgressBar;
use crate::feed::FeedItem;
use crate::sanitize::sanitize_html;
use crate::throttle::HostTimes;

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

/// Fetches and parses a single feed item into a `ScrapedArticle`.
///
/// `log_pb` is any `ProgressBar` belonging to the active `MultiProgress`;
/// errors are routed through `log_pb.println()` so they appear above the bars
/// without corrupting the cursor-tracking used for in-place updates.
///
/// Returns `None` only on HTTP error (the URL will not be cached and will be
/// retried on the next run).  On Readability failure the article is still
/// returned with `html: None` so the URL is cached and not re-fetched.
pub async fn scrape_article(
    client: &reqwest::Client,
    item: FeedItem,
    sanitizer: Arc<Builder<'static>>,
    times: &HostTimes,
    log_pb: ProgressBar,
) -> Option<ScrapedArticle> {
    let html = match crate::throttle::throttled_get(client, &item.url, times).await {
        Err(e) => {
            log_pb.println(format!("HTTP fetch error ({}): {}", item.url, e));
            return None;
        }
        Ok(resp) => match resp.text().await {
            Err(e) => {
                log_pb.println(format!("HTTP read error ({}): {}", item.url, e));
                return None;
            }
            Ok(h) => h,
        },
    };

    let url_str = item.url.clone();

    // Article contains StrTendril (!Send), so convert to owned types inside
    // the blocking thread before returning across the thread boundary.
    let parse_result = tokio::task::spawn_blocking(move || {
        let html = crate::images::sanitize_data_attrs(&html);
        Readability::new(html, Some(&url_str), None)
            .and_then(|mut r| r.parse())
            .map(|a| (a.title, a.byline, a.published_time, a.content.to_string()))
    })
    .await
    .ok()         // JoinError → None
    .and_then(|r| r.ok()); // ReadabilityError → None

    // Readability failure: still return Some so the URL is cached and not
    // re-fetched on every run. The EPUB chapter will have an empty body.
    Some(match parse_result {
        Some((title, byline, published_time, content)) => ScrapedArticle {
            url: item.url,
            title: Some(title).filter(|t| !t.is_empty()).or(item.title),
            author: byline.or(item.author),
            date: published_time.as_deref().and_then(parse_date).or(item.date),
            html: Some(sanitize_html(&sanitizer, &content)),
        },
        None => {
            log_pb.println(format!(
                "Readability failed ({}), caching with no content", item.url
            ));
            ScrapedArticle {
                url: item.url,
                title: item.title,
                author: item.author,
                date: item.date,
                html: None,
            }
        }
    })
}
