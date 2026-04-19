use std::sync::Arc;
use ammonia::Builder;
use chrono::{DateTime, Utc};
use dom_query::Document;
use dom_smoothie::Readability;
use crate::feed::FeedItem;
use crate::log::LogSink;
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

pub async fn scrape_article(
    client: &reqwest::Client,
    item: FeedItem,
    sanitizer: Arc<Builder<'static>>,
    times: &HostTimes,
    log: LogSink,
    content_selectors: Option<Arc<Vec<String>>>,
    remove_selectors: Option<Arc<Vec<String>>>,
) -> Option<ScrapedArticle> {
    let html = match crate::throttle::throttled_get(client, &item.url, times).await {
        Err(e) => {
            log.println(&format!("HTTP fetch error ({}): {}", item.url, e));
            return None;
        }
        Ok(resp) => match resp.text().await {
            Err(e) => {
                log.println(&format!("HTTP read error ({}): {}", item.url, e));
                return None;
            }
            Ok(h) => h,
        },
    };

    if let Some(content_sels) = content_selectors {
        let remove_sels = remove_selectors;
        let html_copy = html.clone();
        let custom_html = tokio::task::spawn_blocking(move || {
            let html = crate::images::sanitize_data_attrs(&html_copy);
            let doc = Document::from(html.as_str());
            if let Some(rm) = remove_sels {
                for sel in rm.iter() {
                    doc.select(sel).remove();
                }
            }
            let mut extracted = String::new();
            for sel in content_sels.iter() {
                for node in doc.select(sel).iter() {
                    extracted.push_str(&node.html().to_string());
                    extracted.push('\n');
                }
            }
            extracted
        })
        .await
        .unwrap_or_default();

        if !custom_html.is_empty() {
            return Some(ScrapedArticle {
                url:    item.url,
                title:  item.title,
                author: item.author,
                date:   item.date,
                html:   Some(sanitize_html(&sanitizer, &custom_html)),
            });
        }
    }

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
            log.println(&format!(
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
