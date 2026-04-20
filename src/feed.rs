use chrono::{DateTime, Utc};
use crate::error::AppError;

pub struct FeedItem {
    pub url: String,
    pub title: Option<String>,
    pub author: Option<String>,
    pub date: Option<DateTime<Utc>>,
}

pub struct FeedData {
    pub title: String,
    pub date:  Option<DateTime<Utc>>,
    pub items: Vec<FeedItem>,
}

pub async fn fetch_feed(
    client: &reqwest::Client,
    url: &str,
) -> Result<FeedData, AppError> {
    let bytes = client.get(url).send().await?.bytes().await?;
    let feed = feed_rs::parser::parse(bytes.as_ref())
        .map_err(|e| AppError::Feed(e.to_string()))?;

    let feed_title = feed
        .title
        .map(|t| t.content)
        .unwrap_or_else(|| "Feed".to_string());

    let items: Vec<FeedItem> = feed
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

    // Prefer feed-level date; fall back to the most recent article date.
    let feed_date = feed.published.or(feed.updated)
        .or_else(|| items.iter().filter_map(|i| i.date).max());

    Ok(FeedData { title: feed_title, date: feed_date, items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use httpmock::prelude::*;

    const RSS_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <channel>
    <title>Test RSS Feed</title>
    <link>https://example.com</link>
    <description>A test feed</description>
    <item>
      <title>Article One</title>
      <link>https://example.com/article-1</link>
      <dc:creator>John Doe</dc:creator>
      <pubDate>Mon, 15 Jan 2024 12:00:00 +0000</pubDate>
    </item>
    <item>
      <title>Article Two</title>
      <link>https://example.com/article-2</link>
    </item>
  </channel>
</rss>"#;

    const ATOM_FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Test Atom Feed</title>
  <entry>
    <title>Atom Article</title>
    <link href="https://example.com/atom-article"/>
    <author><name>Jane Doe</name></author>
    <published>2024-01-15T12:00:00Z</published>
  </entry>
</feed>"#;

    fn test_client() -> reqwest::Client {
        reqwest::Client::new()
    }

    #[tokio::test]
    async fn fetch_rss_feed_parses_title_and_items() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/feed.xml");
            then.status(200)
                .header("content-type", "application/rss+xml")
                .body(RSS_FEED);
        });
        let client = test_client();
        let result = fetch_feed(&client, &server.url("/feed.xml")).await;
        assert!(result.is_ok(), "fetch_feed failed");
        let data = result.unwrap();
        assert_eq!(data.title, "Test RSS Feed");
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].url, "https://example.com/article-1");
        assert_eq!(data.items[0].title.as_deref(), Some("Article One"));
        assert_eq!(data.items[0].author.as_deref(), Some("John Doe"));
        assert!(data.items[0].date.is_some());
        assert_eq!(data.items[1].url, "https://example.com/article-2");
    }

    #[tokio::test]
    async fn fetch_atom_feed_parses_correctly() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/atom.xml");
            then.status(200)
                .header("content-type", "application/atom+xml")
                .body(ATOM_FEED);
        });
        let client = test_client();
        let data = fetch_feed(&client, &server.url("/atom.xml")).await.unwrap();
        assert_eq!(data.title, "Test Atom Feed");
        assert_eq!(data.items.len(), 1);
        assert_eq!(data.items[0].url, "https://example.com/atom-article");
        assert_eq!(data.items[0].author.as_deref(), Some("Jane Doe"));
    }

    #[tokio::test]
    async fn fetch_feed_empty_channel_returns_no_items() {
        let empty_feed = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>Empty</title></channel></rss>"#;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/empty.xml");
            then.status(200).body(empty_feed);
        });
        let client = test_client();
        let data = fetch_feed(&client, &server.url("/empty.xml")).await.unwrap();
        assert!(data.items.is_empty());
    }

    #[tokio::test]
    async fn fetch_feed_missing_title_uses_default() {
        let no_title_feed = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><item><link>https://example.com/a</link></item></channel></rss>"#;
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/notitle.xml");
            then.status(200).body(no_title_feed);
        });
        let client = test_client();
        let data = fetch_feed(&client, &server.url("/notitle.xml")).await.unwrap();
        assert_eq!(data.title, "Feed");
    }

    #[tokio::test]
    async fn fetch_feed_http_error_returns_err() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/error.xml");
            then.status(500);
        });
        let client = test_client();
        let result = fetch_feed(&client, &server.url("/error.xml")).await;
        assert!(result.is_err());
    }
}
