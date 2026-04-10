use std::sync::Arc;
use article_scraper::ArticleScraper;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use url::Url;
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

pub async fn scrape_articles(
    client: &reqwest::Client,
    items: Vec<FeedItem>,
) -> Vec<ScrapedArticle> {
    if items.is_empty() {
        return Vec::new();
    }

    let scraper = Arc::new(ArticleScraper::new(None).await);
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
            let scraper = Arc::clone(&scraper);
            let pb = pb.clone();
            async move {
                let result: Result<ScrapedArticle, AppError> = async {
                    let parsed_url = Url::parse(&item.url)?;
                    let article = scraper
                        .parse(&parsed_url, &client)
                        .await
                        .map_err(|e| AppError::Scraper(e.to_string()))?;
                    Ok(ScrapedArticle {
                        url: item.url,
                        title: article.title.or(item.title),
                        author: article.author.or(item.author),
                        date: article.date.or(item.date),
                        html: article.html,
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
