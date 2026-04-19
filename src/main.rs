mod cache;
mod cli;
mod config;
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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use clap::Parser;
use cli::Args;
use config::{RawDefaults, RawFeed, ResolvedFeedConfig};
use console::style;
use error::AppError;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use log::LogSink;
use tokio::sync::{Mutex, Semaphore};

fn resolve_db_path(cfg: &ResolvedFeedConfig) -> Result<PathBuf, AppError> {
    match &cfg.dbpath {
        None => cache::db_path(),
        Some(p) => {
            let p = PathBuf::from(p);
            Ok(if p.is_dir() { p.join("feedbook.sql") } else { p })
        }
    }
}

async fn run_feed(
    cfg: &ResolvedFeedConfig,
    client: &reqwest::Client,
    conn: &mut rusqlite::Connection,
) -> Result<(), AppError> {
    let t_start = std::time::Instant::now();

    // Per-feed prune (TTL + per-feed cap)
    cache::prune(conn, &cfg.url)?;
    let cached_urls: HashSet<String> = cache::get_cached_urls(conn, &cfg.url)?;

    // Fetch and parse feed
    let report_times = cfg.report_times;

    let t = std::time::Instant::now();
    let feed_data = feed::fetch_feed(client, &cfg.url).await?;
    if report_times { eprintln!("[TIMING] feed fetch: {:?}", t.elapsed()); }
    // Apply name override: cfg.name replaces feed's self-reported title
    let feed_title = cfg.name.clone().unwrap_or(feed_data.title);
    let feed_date  = feed_data.date;
    if cfg.stdout {
        eprintln!("Feed: {}", feed_title);
    }
    let mut feed_items = feed_data.items;
    if let Some(n) = cfg.limit {
        feed_items.truncate(n);
    }

    // Partition into items that need fetching vs those already in cache
    let feed_item_count = feed_items.len();
    let to_fetch: Vec<_> = feed_items
        .into_iter()
        .filter(|item| cfg.force || !cached_urls.contains(&item.url))
        .collect();
    let cached_count = feed_item_count - to_fetch.len();

    // ── Shared pipeline state ─────────────────────────────────────────────────

    let host_times = throttle::new_host_times();
    let image_sem  = Arc::new(Semaphore::new(if cfg!(all(target_arch = "arm", target_env = "musl")) { 3 } else { 8 }));
    let sanitizer  = sanitize::build_sanitizer();

    let cached_sha1s: HashSet<String> = if cfg.no_images {
        HashSet::new()
    } else {
        cache::get_cached_image_sha1s(conn)?
    };
    let seen_sha1s = Arc::new(Mutex::new(cached_sha1s));

    let article_fetched = Arc::new(AtomicU64::new(0));
    let img_fetched     = Arc::new(AtomicU64::new(0));
    let img_hits        = Arc::new(AtomicU64::new(0));

    // ── MultiProgress bars (skipped in --stdout mode) ─────────────────────────
    let mp: Option<MultiProgress>;
    let article_pb: Option<ProgressBar>;
    let image_pb: Option<ProgressBar>;

    if cfg.stdout {
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
        let ipb: Option<ProgressBar> = if cfg.no_images || to_fetch.is_empty() {
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

    // ── Start favicon fetch immediately ──────────────────────────────────────

    // Use cfg.name as the cover title if provided, otherwise derive from URL
    let domain_title    = cfg.name.clone().unwrap_or_else(|| cover::extract_domain_title(&cfg.url));
    let client_favicon  = client.clone();
    let url_for_favicon = cfg.url.clone();
    let favicon_handle  = tokio::spawn(async move {
        cover::fetch_favicon(&client_favicon, &url_for_favicon).await
    });

    // ── Run article+image pipeline concurrently with cover generation ─────────

    let img_concurrency     = if cfg!(all(target_arch = "arm", target_env = "musl")) { 2usize } else { 4 };
    let article_concurrency = if cfg!(all(target_arch = "arm", target_env = "musl")) { 2usize } else { 5 };

    // Include name in cover key so a name change busts the cached cover
    let cover_key = format!("{}|{}|{}", cfg.url, cfg.name.as_deref().unwrap_or(""),
        feed_date.map(|d| d.to_rfc3339()).unwrap_or_default());
    let cover_from_cache = cache::get_cached_cover(conn, &cover_key)?;
    let had_cached_cover = cover_from_cache.is_some();

    let no_images = cfg.no_images;
    let max_w     = cfg.max_image_width;
    let stdout    = cfg.stdout;
    let content_selectors: Option<Arc<Vec<String>>> =
        cfg.content_selectors.as_ref().map(|v| Arc::new(v.clone()));
    let remove_selectors: Option<Arc<Vec<String>>> =
        cfg.remove_selectors.as_ref().map(|v| Arc::new(v.clone()));

    let t_pipeline = std::time::Instant::now();
    let (pipeline_result, cover_png) = tokio::join!(

        // ── Arm A: per-article scrape → per-article image download ────────────
        {
            let client              = client.clone();
            let host_times          = host_times.clone();
            let image_sem           = image_sem.clone();
            let seen_sha1s          = seen_sha1s.clone();
            let sanitizer           = sanitizer.clone();
            let article_pb          = article_pb.clone();
            let image_pb            = image_pb.clone();
            let article_fetched     = article_fetched.clone();
            let img_fetched         = img_fetched.clone();
            let img_hits            = img_hits.clone();
            let content_selectors   = content_selectors.clone();
            let remove_selectors    = remove_selectors.clone();
            async move {
                let results: Vec<(scraper::ScrapedArticle, Vec<images::ProcessedImage>)> =
                    futures::stream::iter(to_fetch)
                        .map({
                            let client              = client.clone();
                            let host_times          = host_times.clone();
                            let image_sem           = image_sem.clone();
                            let seen_sha1s          = seen_sha1s.clone();
                            let sanitizer           = sanitizer.clone();
                            let article_pb          = article_pb.clone();
                            let image_pb            = image_pb.clone();
                            let article_fetched     = article_fetched.clone();
                            let img_fetched         = img_fetched.clone();
                            let img_hits            = img_hits.clone();
                            let content_selectors   = content_selectors.clone();
                            let remove_selectors    = remove_selectors.clone();
                            move |item| {
                                let client              = client.clone();
                                let host_times          = host_times.clone();
                                let image_sem           = image_sem.clone();
                                let seen_sha1s          = seen_sha1s.clone();
                                let sanitizer           = sanitizer.clone();
                                let article_pb          = article_pb.clone();
                                let image_pb            = image_pb.clone();
                                let article_fetched     = article_fetched.clone();
                                let img_fetched         = img_fetched.clone();
                                let img_hits            = img_hits.clone();
                                let content_selectors   = content_selectors.clone();
                                let remove_selectors    = remove_selectors.clone();
                                async move {
                                    let log = match article_pb.as_ref() {
                                        Some(pb) => LogSink::Bar(pb.clone()),
                                        None     => LogSink::Stderr,
                                    };

                                    let item_url = item.url.clone();
                                    let maybe_article = scraper::scrape_article(
                                        &client, item, sanitizer, &host_times, log.clone(),
                                        content_selectors, remove_selectors,
                                    ).await;

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
                                            .buffer_unordered(img_concurrency)
                                            .filter_map(|r| async move { r })
                                            .collect::<Vec<_>>()
                                            .await
                                    };

                                    Some((article, article_images))
                                }
                            }
                        })
                        .buffer_unordered(article_concurrency)
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
            async move {
                let cover_sp = mp_cover.as_ref().map(|m| {
                    let sp = m.add(ProgressBar::new_spinner());
                    sp.set_style(
                        ProgressStyle::with_template("{spinner:.yellow} {msg}").unwrap()
                    );
                    sp.set_message("Fetching favicon...");
                    sp
                });

                if let Some(cached) = cover_from_cache {
                    if let Some(sp) = cover_sp { sp.finish_with_message("Cover cached"); }
                    else if stdout { eprintln!("Cover cached"); }
                    if report_times { eprintln!("[TIMING] cover: cached (skipped generation)"); }
                    return Some(cached);
                }

                let t_favicon = std::time::Instant::now();
                let favicon = favicon_handle.await.ok().flatten();
                if report_times { eprintln!("[TIMING] favicon fetch: {:?}", t_favicon.elapsed()); }

                if let Some(ref sp) = cover_sp {
                    sp.set_message("Generating cover...");
                } else if stdout {
                    eprintln!("Generating cover...");
                }
                let title_owned = domain_title;
                let t_cover = std::time::Instant::now();
                let result = tokio::task::spawn_blocking(move || {
                    cover::generate_cover(&title_owned, feed_date, favicon.as_deref())
                })
                .await
                .ok()
                .and_then(|r| r.ok());
                if report_times { eprintln!("[TIMING] cover generate: {:?}", t_cover.elapsed()); }

                if let Some(sp) = cover_sp {
                    sp.finish_with_message("Cover ready");
                } else if stdout {
                    eprintln!("Cover ready");
                }
                result
            }
        },
    );

    if report_times { eprintln!("[TIMING] pipeline (articles + cover, concurrent): {:?}", t_pipeline.elapsed()); }

    // ── Store newly generated cover in cache ──────────────────────────────────

    if !had_cached_cover {
        if let Some(ref png) = cover_png {
            let _ = cache::store_cover(conn, &cover_key, png);
        }
    }

    // ── Batch DB inserts (all on the main task — rusqlite is !Send) ───────────

    let t = std::time::Instant::now();
    let tx = conn.transaction()?;
    for (article, article_images) in &pipeline_result {
        for img in article_images {
            cache::insert_image(&*tx, img)?;
        }
        cache::insert_article(&*tx, &cfg.url, article)?;
    }
    tx.commit()?;
    if report_times { eprintln!("[TIMING] db inserts ({} articles): {:?}", pipeline_result.len(), t.elapsed()); }

    // ── Load from DB (respects limit) ────────────────────────────────────────

    let t = std::time::Instant::now();
    let all_articles = cache::load_articles(conn, &cfg.url, cfg.limit)?;
    if report_times { eprintln!("[TIMING] db load ({} articles): {:?}", all_articles.len(), t.elapsed()); }

    if all_articles.is_empty() {
        eprintln!("No articles found.");
        return Ok(());
    }

    // Collect the images referenced by the EPUB articles
    let epub_images: HashMap<String, images::ProcessedImage> = if cfg.no_images {
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
        cache::load_images(conn, &sha1s)?
            .into_iter()
            .map(|img| (img.url_sha1.clone(), img))
            .collect()
    };

    // ── Build EPUB / KEPUB ────────────────────────────────────────────────────

    let epub_sp = mp.as_ref().map(|m| {
        let sp = m.add(ProgressBar::new_spinner());
        sp.set_style(ProgressStyle::with_template("{spinner:.green} {msg}").unwrap());
        sp.set_message(format!("Building {} ({} articles)...",
            if cfg.kobo { "KEPUB" } else { "EPUB" }, all_articles.len()));
        sp
    });
    if epub_sp.is_none() && cfg.stdout {
        eprintln!("Building {} ({} articles)...",
            if cfg.kobo { "KEPUB" } else { "EPUB" }, all_articles.len());
    }

    let output_path = epub::derive_output_path(&feed_title, cfg.kobo);
    let output_path = if let Some(ref folder) = cfg.outfolder {
        let dir = PathBuf::from(folder);
        std::fs::create_dir_all(&dir)?;
        dir.join(output_path)
    } else {
        output_path
    };
    let output_display = output_path.display().to_string();
    let kobo = cfg.kobo;

    let t = std::time::Instant::now();
    let epub_result = tokio::task::spawn_blocking(move || {
        epub::build_epub(&feed_title, &all_articles, &epub_images, cover_png, &output_path, kobo)
    })
    .await
    .map_err(|e| AppError::Other(format!("EPUB task panicked: {e}")))?;

    epub_result?;
    if report_times {
        eprintln!("[TIMING] epub build: {:?}", t.elapsed());
        eprintln!("[TIMING] total: {:?}", t_start.elapsed());
    }

    if let Some(sp) = epub_sp {
        sp.finish_with_message(format!("Written: {}", output_display));
    } else {
        eprintln!("Written: {}", output_display);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let args = Args::parse();

    let config_result = config::load_config(args.config.as_deref())?;

    let (feeds_to_run, db_path): (Vec<ResolvedFeedConfig>, PathBuf) = match &config_result {
        None => {
            let url = args.url.clone().ok_or(AppError::NoUrl)?;
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let cfg = config::merge(&args, &RawDefaults::default(), &RawFeed::ad_hoc(url), &cwd);
            let db = resolve_db_path(&cfg)?;
            (vec![cfg], db)
        }
        Some((raw_config, config_path)) => {
            let config_dir = config_path.parent().unwrap_or(Path::new("."));
            let defaults = raw_config.defaults.as_ref().cloned().unwrap_or_default();

            let feeds: Vec<ResolvedFeedConfig> = if let Some(url) = &args.url {
                let feed = raw_config.feeds.iter().find(|f| &f.url == url);
                let f = feed.cloned().unwrap_or_else(|| RawFeed::ad_hoc(url.clone()));
                vec![config::merge(&args, &defaults, &f, config_dir)]
            } else {
                raw_config.feeds.iter()
                    .filter(|f| f.enabled.unwrap_or(true))
                    .map(|f| config::merge(&args, &defaults, f, config_dir))
                    .collect()
            };

            if feeds.is_empty() {
                eprintln!("No feeds to process.");
                return Ok(());
            }

            let db = resolve_db_path(&feeds[0])?;
            (feeds, db)
        }
    };

    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/146.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()?;

    let mut conn = cache::open_db(&db_path)?;
    cache::prune_images(&conn)?;

    for cfg in &feeds_to_run {
        if let Err(e) = run_feed(cfg, &client, &mut conn).await {
            eprintln!("Error processing {}: {e}", cfg.url);
        }
    }

    Ok(())
}
