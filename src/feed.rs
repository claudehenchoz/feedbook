use chrono::{DateTime, Utc};
use crate::error::AppError;

pub struct FeedItem {
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<DateTime<Utc>>,
}

pub async fn fetch_feed(
    client: &reqwest::Client,
    url: &str,
) -> Result<(String, Vec<FeedItem>), AppError> {
    let bytes = client.get(url).send().await?.bytes().await?;
    let feed = feed_rs::parser::parse(bytes.as_ref())
        .map_err(|e| AppError::Feed(e.to_string()))?;

    let feed_title = feed
        .title
        .map(|t| t.content)
        .unwrap_or_else(|| "Feed".to_string());

    let items = feed
        .entries
        .into_iter()
        .filter_map(|entry| {
            let url = entry.links.into_iter().next()?.href;
            Some(FeedItem {
                url,
                title: entry.title.map(|t| t.content),
                author: entry.authors.into_iter().next().map(|a| a.name),
                date: entry.published.or(entry.updated).map(|dt| dt.with_timezone(&Utc)),
            })
        })
        .collect();

    Ok((feed_title, items))
}
