mod cache;
mod cli;
mod epub;
mod error;
mod feed;
mod scraper;

use std::collections::HashSet;
use std::time::Duration;
use clap::Parser;
use cli::Args;
use error::AppError;

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let args = Args::parse();

    let client = reqwest::Client::builder()
        .user_agent("feedbook/0.1")
        .timeout(Duration::from_secs(30))
        .build()?;

    // Open DB, prune stale entries, get already-cached URLs
    let db_path = cache::db_path()?;
    let conn = cache::open_db(&db_path)?;
    cache::prune(&conn, &args.url)?;
    let cached_urls: HashSet<String> = cache::get_cached_urls(&conn, &args.url)?;

    // Fetch and parse feed
    let (feed_title, mut feed_items) = feed::fetch_feed(&client, &args.url).await?;
    if let Some(n) = args.limit {
        feed_items.truncate(n);
    }

    // Only fetch articles not already in cache
    let to_fetch: Vec<_> = feed_items
        .into_iter()
        .filter(|item| !cached_urls.contains(&item.url))
        .collect();

    if !to_fetch.is_empty() {
        println!("Fetching {} new article(s)...", to_fetch.len());
        let new_articles = scraper::scrape_articles(&client, to_fetch).await;
        for article in &new_articles {
            cache::insert_article(&conn, &args.url, article)?;
        }
    }

    // Load all articles for this feed from cache (respects limit)
    let all_articles = cache::load_articles(&conn, &args.url, args.limit)?;

    if all_articles.is_empty() {
        eprintln!("No articles found.");
        return Ok(());
    }

    println!("Building EPUB with {} article(s)...", all_articles.len());
    let output_path = epub::derive_output_path(&feed_title);
    epub::build_epub(&feed_title, &all_articles, &output_path)?;
    println!("Written: {}", output_path.display());

    Ok(())
}
