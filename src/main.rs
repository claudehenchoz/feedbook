mod cache;
mod cli;
mod cover;
mod epub;
mod error;
mod feed;
mod images;
mod sanitize;
mod scraper;

use std::collections::{HashMap, HashSet};
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
    cache::prune_images(&conn)?;
    let cached_urls: HashSet<String> = cache::get_cached_urls(&conn, &args.url)?;

    // Fetch and parse feed
    let feed_data = feed::fetch_feed(&client, &args.url).await?;
    let feed_title = feed_data.title;
    let feed_date  = feed_data.date;
    let mut feed_items = feed_data.items;
    if let Some(n) = args.limit {
        feed_items.truncate(n);
    }

    // Only fetch articles not already in cache (unless --force)
    let to_fetch: Vec<_> = feed_items
        .into_iter()
        .filter(|item| args.force || !cached_urls.contains(&item.url))
        .collect();

    if !to_fetch.is_empty() {
        println!("Fetching {} new article(s)...", to_fetch.len());
        let new_articles = scraper::scrape_articles(&client, to_fetch).await;

        // Download and cache images for newly scraped articles
        if !args.no_images {
            let cached_sha1s = cache::get_cached_image_sha1s(&conn)?;

            // Collect all unique (raw_src, absolute_url) pairs not yet in image cache
            let mut seen_abs: HashSet<String> = HashSet::new();
            let uncached_urls: Vec<(String, String)> = new_articles
                .iter()
                .filter_map(|a| a.html.as_deref().zip(Some(a.url.as_str())))
                .flat_map(|(html, url)| images::extract_image_urls(html, url))
                .filter(|(_, abs)| {
                    !cached_sha1s.contains(&images::url_sha1(abs))
                        && seen_abs.insert(abs.clone())
                })
                .collect();

            if !uncached_urls.is_empty() {
                println!("Fetching {} image(s)...", uncached_urls.len());
                let new_images = images::download_and_process(
                    &client,
                    uncached_urls,
                    args.max_image_width,
                )
                .await;
                for img in &new_images {
                    cache::insert_image(&conn, img)?;
                }
            }
        }

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

    // Load images referenced by the articles that will go into the EPUB
    let epub_images: HashMap<String, images::ProcessedImage> = if args.no_images {
        HashMap::new()
    } else {
        let sha1s: Vec<String> = {
            let mut seen: HashSet<String> = HashSet::new();
            all_articles
                .iter()
                .filter_map(|a| a.html.as_deref().zip(Some(a.url.as_str())))
                .flat_map(|(html, url)| images::extract_image_urls(html, url))
                .map(|(_, abs)| images::url_sha1(&abs))
                .filter(|s| seen.insert(s.clone()))
                .collect()
        };
        cache::load_images(&conn, &sha1s)?
            .into_iter()
            .map(|img| (img.url_sha1.clone(), img))
            .collect()
    };

    // Generate cover image
    let domain_title = cover::extract_domain_title(&args.url);
    let favicon_bytes = cover::fetch_favicon(&client, &args.url).await;
    let cover_png = match cover::generate_cover(&domain_title, feed_date, favicon_bytes.as_deref()) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            eprintln!("Cover generation failed: {e}");
            None
        }
    };

    println!("Building EPUB with {} article(s)...", all_articles.len());
    let output_path = epub::derive_output_path(&feed_title);
    epub::build_epub(&feed_title, &all_articles, &epub_images, cover_png, &output_path)?;
    println!("Written: {}", output_path.display());

    Ok(())
}
