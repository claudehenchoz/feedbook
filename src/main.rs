mod cache;
mod cli;
mod cover;
mod epub;
mod error;
mod feed;
mod images;
mod log;
mod sanitize;
mod scraper;
mod throttle;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use clap::Parser;
use cli::Args;
use console::style;
use error::AppError;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::LogSink;
use tokio::sync::{Mutex, Semaphore};

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let args = Args::parse();

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(30))
        .build()?;

    // Open DB, prune stale entries, get already-cached URLs
    let db_path = match &args.dbpath {
        None => cache::db_path()?,
        Some(p) => {
            let p = std::path::PathBuf::from(p);
            if p.is_dir() { p.join("feedbook.sql") } else { p }
        }
    };
    let conn = cache::open_db(&db_path)?;
    cache::prune(&conn, &args.url)?;
    cache::prune_images(&conn)?;
    let cached_urls: HashSet<String> = cache::get_cached_urls(&conn, &args.url)?;

    // Fetch and parse feed
    let feed_data = feed::fetch_feed(&client, &args.url).await?;
    let feed_title = feed_data.title;
    let feed_date  = feed_data.date;
    if args.stdout {
        eprintln!("Feed: {}", feed_title);
    }
    let mut feed_items = feed_data.items;
    if let Some(n) = args.limit {
        feed_items.truncate(n);
    }

    // Partition into items that need fetching vs those already in cache
    let feed_item_count = feed_items.len();
    let to_fetch: Vec<_> = feed_items
        .into_iter()
        .filter(|item| args.force || !cached_urls.contains(&item.url))
        .collect();
    let cached_count = feed_item_count - to_fetch.len();

    // ── Shared pipeline state ─────────────────────────────────────────────────

    let host_times = throttle::new_host_times();
    // Global cap on concurrent image downloads + processing (per Semaphore permit)
    let image_sem  = Arc::new(Semaphore::new(8));
    let sanitizer  = sanitize::build_sanitizer();

    // Image SHA1 dedup: start with what is already in the DB
    let cached_sha1s: HashSet<String> = if args.no_images {
        HashSet::new()
    } else {
        cache::get_cached_image_sha1s(&conn)?
    };
    let seen_sha1s = Arc::new(Mutex::new(cached_sha1s));

    // Live counters for progress bar messages
    let article_fetched = Arc::new(AtomicU64::new(0));
    let img_fetched     = Arc::new(AtomicU64::new(0));
    let img_hits        = Arc::new(AtomicU64::new(0));

    // ── MultiProgress bars (skipped in --stdout mode) ─────────────────────────
    let mp: Option<MultiProgress>;
    let article_pb: Option<ProgressBar>;
    let image_pb: Option<ProgressBar>;

    if args.stdout {
        mp         = None;
        article_pb = None;
        image_pb   = None;
    } else {
        let m = MultiProgress::with_draw_target(
            indicatif::ProgressDrawTarget::stderr_with_hz(8),
        );
        let apb = m.add(ProgressBar::new(feed_item_count as u64));
        apb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} articles  {msg}"
            )
            .unwrap()
            .progress_chars("#>-"),
        );
        if cached_count > 0 {
            apb.inc(cached_count as u64);
            apb.set_message(style(format!("{} cached", cached_count)).dim().to_string());
        }
        let ipb: Option<ProgressBar> = if args.no_images || to_fetch.is_empty() {
            None
        } else {
            let pb = m.add(ProgressBar::new(0));
            pb.set_style(
                ProgressStyle::with_template(
                    "{spinner:.cyan} [{bar:40.magenta/blue}] {pos}/{len} images  {msg}"
                )
                .unwrap()
                .progress_chars("#>-"),
            );
            Some(pb)
        };
        mp         = Some(m);
        article_pb = Some(apb);
        image_pb   = ipb;
    };

    // ── Start favicon fetch immediately (only needs the feed URL) ─────────────

    let domain_title    = cover::extract_domain_title(&args.url);
    let client_favicon  = client.clone();
    let url_for_favicon = args.url.clone();
    let favicon_handle  = tokio::spawn(async move {
        cover::fetch_favicon(&client_favicon, &url_for_favicon).await
    });

    // ── Run article+image pipeline concurrently with cover generation ─────────

    let (pipeline_result, cover_png) = tokio::join!(

        // ── Arm A: per-article scrape → per-article image download ────────────
        {
            let client          = client.clone();
            let host_times      = host_times.clone();
            let image_sem       = image_sem.clone();
            let seen_sha1s      = seen_sha1s.clone();
            let sanitizer       = sanitizer.clone();
            let article_pb      = article_pb.clone();
            let image_pb        = image_pb.clone();
            let article_fetched = article_fetched.clone();
            let img_fetched     = img_fetched.clone();
            let img_hits        = img_hits.clone();
            let no_images       = args.no_images;
            let max_w           = args.max_image_width;
            let stdout          = args.stdout;
            async move {
                let results: Vec<(scraper::ScrapedArticle, Vec<images::ProcessedImage>)> =
                    futures::stream::iter(to_fetch)
                        .map({
                            let client          = client.clone();
                            let host_times      = host_times.clone();
                            let image_sem       = image_sem.clone();
                            let seen_sha1s      = seen_sha1s.clone();
                            let sanitizer       = sanitizer.clone();
                            let article_pb      = article_pb.clone();
                            let image_pb        = image_pb.clone();
                            let article_fetched = article_fetched.clone();
                            let img_fetched     = img_fetched.clone();
                            let img_hits        = img_hits.clone();
                            move |item| {
                                let client          = client.clone();
                                let host_times      = host_times.clone();
                                let image_sem       = image_sem.clone();
                                let seen_sha1s      = seen_sha1s.clone();
                                let sanitizer       = sanitizer.clone();
                                let article_pb      = article_pb.clone();
                                let image_pb        = image_pb.clone();
                                let article_fetched = article_fetched.clone();
                                let img_fetched     = img_fetched.clone();
                                let img_hits        = img_hits.clone();
                                async move {
                                    let log = match article_pb.as_ref() {
                                        Some(pb) => LogSink::Bar(pb.clone()),
                                        None     => LogSink::Stderr,
                                    };

                                    let item_url = item.url.clone();
                                    let maybe_article = scraper::scrape_article(
                                        &client, item, sanitizer, &host_times, log.clone(),
                                    ).await;

                                    // Update article progress or log to stdout
                                    if stdout {
                                        let label = maybe_article.as_ref()
                                            .and_then(|a| a.title.as_deref())
                                            .unwrap_or(&item_url);
                                        eprintln!("Article: {}", label);
                                    } else if let Some(ref pb) = article_pb {
                                        pb.inc(1);
                                        let fetched = article_fetched.fetch_add(1, Ordering::Relaxed) + 1;
                                        pb.set_message(format!(
                                            "{}  {}",
                                            style(format!("{} cached", cached_count)).dim(),
                                            style(format!("{} fetched", fetched)).cyan(),
                                        ));
                                    }

                                    let article = maybe_article?;

                                    // ── Per-article image pipeline ────────────
                                    let article_images = if no_images || article.html.is_none() {
                                        vec![]
                                    } else {
                                        let img_urls = images::extract_image_urls(
                                            article.html.as_deref().unwrap_or(""),
                                            &article.url,
                                        );

                                        // Partition into already-seen (cache hit) vs new
                                        let (hit_count, to_download) = {
                                            let mut seen = seen_sha1s.lock().await;
                                            let mut hits = 0u64;
                                            let mut new_urls = Vec::new();
                                            for pair in img_urls {
                                                if seen.insert(images::url_sha1(&pair.1)) {
                                                    new_urls.push(pair);
                                                } else {
                                                    hits += 1;
                                                }
                                            }
                                            (hits, new_urls)
                                        };

                                        if hit_count > 0 {
                                            let prev = img_hits.fetch_add(hit_count, Ordering::Relaxed);
                                            if let Some(ref pb) = image_pb {
                                                let total_hits    = prev + hit_count;
                                                let total_fetched = img_fetched.load(Ordering::Relaxed);
                                                pb.set_message(format!(
                                                    "{}  {}",
                                                    style(format!("{} cached", total_hits)).dim(),
                                                    style(format!("{} fetched", total_fetched)).cyan(),
                                                ));
                                            }
                                        }
                                        if !to_download.is_empty() {
                                            if let Some(ref pb) = image_pb {
                                                pb.inc_length(to_download.len() as u64);
                                            }
                                        }

                                        futures::stream::iter(to_download)
                                            .map({
                                                let client      = client.clone();
                                                let host_times  = host_times.clone();
                                                let image_sem   = image_sem.clone();
                                                let image_pb    = image_pb.clone();
                                                let img_fetched = img_fetched.clone();
                                                let img_hits    = img_hits.clone();
                                                let log         = log.clone();
                                                move |(raw_src, abs_url)| {
                                                    let client      = client.clone();
                                                    let host_times  = host_times.clone();
                                                    let image_sem   = image_sem.clone();
                                                    let image_pb    = image_pb.clone();
                                                    let img_fetched = img_fetched.clone();
                                                    let img_hits    = img_hits.clone();
                                                    let log         = log.clone();
                                                    async move {
                                                        let result = images::download_image(
                                                            &client, raw_src, abs_url,
                                                            max_w, &host_times, &image_sem,
                                                            log,
                                                        ).await;
                                                        if let Some(ref pb) = image_pb {
                                                            pb.inc(1);
                                                            let total_fetched = img_fetched.fetch_add(1, Ordering::Relaxed) + 1;
                                                            let total_hits    = img_hits.load(Ordering::Relaxed);
                                                            pb.set_message(format!(
                                                                "{}  {}",
                                                                style(format!("{} cached", total_hits)).dim(),
                                                                style(format!("{} fetched", total_fetched)).cyan(),
                                                            ));
                                                        }
                                                        result
                                                    }
                                                }
                                            })
                                            .buffer_unordered(4)
                                            .filter_map(|r| async move { r })
                                            .collect::<Vec<_>>()
                                            .await
                                    };

                                    Some((article, article_images))
                                }
                            }
                        })
                        .buffer_unordered(5)
                        .filter_map(|r| async move { r })
                        .collect::<Vec<_>>()
                        .await;

                if let Some(ref pb) = article_pb {
                    pb.finish();
                }
                if let Some(ref pb) = image_pb {
                    pb.finish();
                }
                results
            }
        },

        // ── Arm B: favicon fetch + cover generation ───────────────────────────
        {
            let mp_cover     = mp.clone();
            let domain_title = domain_title.clone();
            let stdout       = args.stdout;
            async move {
                let cover_sp = mp_cover.as_ref().map(|m| {
                    let sp = m.add(ProgressBar::new_spinner());
                    sp.set_style(
                        ProgressStyle::with_template("{spinner:.yellow} {msg}").unwrap()
                    );
                    sp.set_message("Fetching favicon...");
                    sp
                });

                let favicon = favicon_handle.await.ok().flatten();

                if let Some(ref sp) = cover_sp {
                    sp.set_message("Generating cover...");
                } else if stdout {
                    eprintln!("Generating cover...");
                }
                let title_owned = domain_title;
                let result = tokio::task::spawn_blocking(move || {
                    cover::generate_cover(&title_owned, feed_date, favicon.as_deref())
                })
                .await
                .ok()
                .and_then(|r| r.ok());

                if let Some(sp) = cover_sp {
                    sp.finish_with_message("Cover ready");
                } else if stdout {
                    eprintln!("Cover ready");
                }
                result
            }
        },
    );

    // ── Batch DB inserts (all on the main task — rusqlite is !Send) ───────────

    for (article, article_images) in &pipeline_result {
        for img in article_images {
            cache::insert_image(&conn, img)?;
        }
        cache::insert_article(&conn, &args.url, article)?;
    }

    // ── Load from DB (respects --limit) ──────────────────────────────────────

    let all_articles = cache::load_articles(&conn, &args.url, args.limit)?;

    if all_articles.is_empty() {
        eprintln!("No articles found.");
        return Ok(());
    }

    // Collect the images referenced by the EPUB articles
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

    // ── Build EPUB / KEPUB ────────────────────────────────────────────────────

    let epub_sp = mp.as_ref().map(|m| {
        let sp = m.add(ProgressBar::new_spinner());
        sp.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
        sp.set_message(format!("Building {} ({} articles)...",
            if args.kobo { "KEPUB" } else { "EPUB" }, all_articles.len()));
        sp
    });
    if epub_sp.is_none() && args.stdout {
        eprintln!("Building {} ({} articles)...",
            if args.kobo { "KEPUB" } else { "EPUB" }, all_articles.len());
    }

    let output_path = epub::derive_output_path(&feed_title, args.kobo);
    let output_path = if let Some(ref folder) = args.outfolder {
        let dir = std::path::PathBuf::from(folder);
        std::fs::create_dir_all(&dir)?;
        dir.join(output_path)
    } else {
        output_path
    };
    let output_display = output_path.display().to_string();
    let kobo = args.kobo;

    let epub_result = tokio::task::spawn_blocking(move || {
        epub::build_epub(&feed_title, &all_articles, &epub_images, cover_png, &output_path, kobo)
    })
    .await
    .map_err(|e| AppError::Other(format!("EPUB task panicked: {e}")))?;

    epub_result?;

    if let Some(sp) = epub_sp {
        sp.finish_with_message(format!("Written: {}", output_display));
    } else {
        eprintln!("Written: {}", output_display);
    }

    Ok(())
}
