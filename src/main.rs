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
use std::time::Duration;
use clap::Parser;
use cli::Args;
use config::{RawDefaults, RawFeed, ResolvedFeedConfig};
use error::AppError;
use futures::StreamExt;
use log::LogSink;
use tokio::sync::{Mutex, Semaphore};

/// Returns the hostname of `url` (e.g. `hnrss.org`) for use as a log line prefix.
fn feed_log_prefix(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.to_owned())
}

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
    log_file: Option<log::LogFile>,
) -> Result<(), AppError> {
    let db_path = resolve_db_path(cfg)?;
    let mut conn = cache::open_db(&db_path)?;
    let t_start = std::time::Instant::now();
    let run_log = LogSink::new(feed_log_prefix(&cfg.url)).with_file_opt(log_file.clone());

    // Per-feed prune (TTL + per-feed cap)
    cache::prune(&conn, &cfg.url)?;
    let cached_urls: HashSet<String> = cache::get_cached_urls(&conn, &cfg.url)?;

    let report_times = cfg.report_times;

    let t = std::time::Instant::now();
    let feed_data = feed::fetch_feed(client, &cfg.url).await?;
    if report_times { run_log.println(&format!("[TIMING] feed fetch: {:?}", t.elapsed())); }
    // Apply name override: cfg.name replaces feed's self-reported title
    let feed_title = cfg.name.clone().unwrap_or(feed_data.title);
    let feed_date  = feed_data.date;
    run_log.println(&format!("Feed: {}", feed_title));

    let mut feed_items = feed_data.items;
    if let Some(n) = cfg.limit {
        feed_items.truncate(n);
    }

    let to_fetch: Vec<_> = feed_items
        .into_iter()
        .filter(|item| cfg.force || !cached_urls.contains(&item.url))
        .collect();

    // ── Shared pipeline state ─────────────────────────────────────────────────

    let host_times = throttle::new_host_times();
    let image_sem  = Arc::new(Semaphore::new(if cfg!(all(target_arch = "arm", target_env = "musl")) { 3 } else { 8 }));
    let sanitizer  = sanitize::build_sanitizer();

    let cached_sha1s: HashSet<String> = if cfg.no_images {
        HashSet::new()
    } else {
        cache::get_cached_image_sha1s(&conn)?
    };
    let seen_sha1s = Arc::new(Mutex::new(cached_sha1s));

    // ── Cover template cache check + conditional favicon spawn ───────────────

    // Use cfg.name as the cover title if provided, otherwise derive from URL
    let domain_title = cfg.name.clone().unwrap_or_else(|| cover::extract_domain_title(&cfg.url));

    // Key covers by feed URL + name only (no date) so the expensive template is
    // reused across runs; only the date/time overlay is re-applied each time.
    let template_key         = format!("{}|{}", cfg.url, cfg.name.as_deref().unwrap_or(""));
    let cover_template_cache = cache::get_cached_cover(&conn, &template_key)?;
    let had_cached_template  = cover_template_cache.is_some();

    // Only fetch the favicon when we actually need to (re)build the template.
    let favicon_handle: Option<tokio::task::JoinHandle<Option<Vec<u8>>>> =
        if !had_cached_template {
            let client_favicon  = client.clone();
            let url_for_favicon = cfg.url.clone();
            Some(tokio::spawn(async move {
                cover::fetch_favicon(&client_favicon, &url_for_favicon).await
            }))
        } else {
            None
        };

    // ── Run article+image pipeline concurrently with cover generation ─────────

    let img_concurrency     = if cfg!(all(target_arch = "arm", target_env = "musl")) { 2usize } else { 4 };
    let article_concurrency = if cfg!(all(target_arch = "arm", target_env = "musl")) { 2usize } else { 5 };

    let no_images = cfg.no_images;
    let max_w     = cfg.max_image_width;
    let content_selectors: Option<Arc<Vec<String>>> =
        cfg.content_selectors.as_ref().map(|v| Arc::new(v.clone()));
    let remove_selectors: Option<Arc<Vec<String>>> =
        cfg.remove_selectors.as_ref().map(|v| Arc::new(v.clone()));

    let t_pipeline = std::time::Instant::now();
    let (pipeline_result, cover_result) = tokio::join!(

        // ── Arm A: per-article scrape → per-article image download ────────────
        {
            let run_log           = run_log.clone();
            let client            = client.clone();
            let host_times        = host_times.clone();
            let image_sem         = image_sem.clone();
            let seen_sha1s        = seen_sha1s.clone();
            let sanitizer         = sanitizer.clone();
            let content_selectors = content_selectors.clone();
            let remove_selectors  = remove_selectors.clone();
            async move {
                let results: Vec<(scraper::ScrapedArticle, Vec<images::ProcessedImage>)> =
                    futures::stream::iter(to_fetch)
                        .map({
                            let run_log           = run_log.clone();
                            let client            = client.clone();
                            let host_times        = host_times.clone();
                            let image_sem         = image_sem.clone();
                            let seen_sha1s        = seen_sha1s.clone();
                            let sanitizer         = sanitizer.clone();
                            let content_selectors = content_selectors.clone();
                            let remove_selectors  = remove_selectors.clone();
                            move |item| {
                                let run_log           = run_log.clone();
                                let client            = client.clone();
                                let host_times        = host_times.clone();
                                let image_sem         = image_sem.clone();
                                let seen_sha1s        = seen_sha1s.clone();
                                let sanitizer         = sanitizer.clone();
                                let content_selectors = content_selectors.clone();
                                let remove_selectors  = remove_selectors.clone();
                                async move {
                                    let item_url = item.url.clone();
                                    let maybe_article = scraper::scrape_article(
                                        &client, item, sanitizer, &host_times, run_log.clone(),
                                        content_selectors, remove_selectors,
                                    ).await;

                                    let label = maybe_article.as_ref()
                                        .and_then(|a| a.title.as_deref())
                                        .unwrap_or(&item_url);
                                    run_log.println(&format!("Article: {}", label));

                                    let article = maybe_article?;

                                    // ── Per-article image pipeline ────────────
                                    let article_images = if no_images || article.html.is_none() {
                                        vec![]
                                    } else {
                                        let img_urls = images::extract_image_urls(
                                            article.html.as_deref().unwrap_or(""),
                                            &article.url,
                                        );

                                        let to_download = {
                                            let mut seen = seen_sha1s.lock().await;
                                            img_urls
                                                .into_iter()
                                                .filter(|(_, u)| seen.insert(images::url_sha1(u)))
                                                .collect::<Vec<_>>()
                                        };

                                        futures::stream::iter(to_download)
                                            .map({
                                                let client     = client.clone();
                                                let host_times = host_times.clone();
                                                let image_sem  = image_sem.clone();
                                                let run_log    = run_log.clone();
                                                move |(raw_src, abs_url)| {
                                                    let client     = client.clone();
                                                    let host_times = host_times.clone();
                                                    let image_sem  = image_sem.clone();
                                                    let run_log    = run_log.clone();
                                                    async move {
                                                        images::download_image(
                                                            &client, raw_src, abs_url,
                                                            max_w, &host_times, &image_sem,
                                                            run_log,
                                                        ).await
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
                results
            }
        },

        // ── Arm B: cover template (cached or generated) + date overlay ───────
        {
            let run_log = run_log.clone();
            async move {
                // ── 1. Get or build the static template ──────────────────────
                let (template, new_template): (Vec<u8>, Option<Vec<u8>>) =
                    if let Some(cached) = cover_template_cache {
                        run_log.println("Cover template cached");
                        if report_times { run_log.println("[TIMING] cover template: cached (skipped generation)"); }
                        (cached, None)
                    } else {
                        let t_favicon = std::time::Instant::now();
                        let favicon = if let Some(h) = favicon_handle { h.await.ok().flatten() } else { None };
                        if report_times { run_log.println(&format!("[TIMING] favicon fetch: {:?}", t_favicon.elapsed())); }

                        run_log.println("Generating cover template...");

                        let title_owned = domain_title;
                        let t_cover = std::time::Instant::now();
                        let tmpl = tokio::task::spawn_blocking(move || {
                            cover::generate_cover_template(&title_owned, favicon.as_deref())
                        })
                        .await
                        .ok()
                        .and_then(|r| r.ok());
                        if report_times { run_log.println(&format!("[TIMING] cover template generate: {:?}", t_cover.elapsed())); }

                        match tmpl {
                            Some(t) => { let saved = t.clone(); (t, Some(saved)) }
                            None    => return None,
                        }
                    };

                // ── 2. Apply date/time overlay (fast: PNG decode + text + encode) ──
                let t_apply = std::time::Instant::now();
                let cover = tokio::task::spawn_blocking(move || {
                    cover::apply_date_to_cover(&template, feed_date)
                })
                .await
                .ok()
                .and_then(|r| r.ok())?;
                if report_times { run_log.println(&format!("[TIMING] cover date apply: {:?}", t_apply.elapsed())); }

                run_log.println("Cover ready");

                Some((cover, new_template))
            }
        },
    );

    if report_times { run_log.println(&format!("[TIMING] pipeline (articles + cover, concurrent): {:?}", t_pipeline.elapsed())); }

    // ── Destructure cover result and persist newly generated template ─────────

    let (cover_png, cover_new_template) = match cover_result {
        Some((cover, tmpl)) => (Some(cover), tmpl),
        None => (None, None),
    };

    if let Some(template) = cover_new_template {
        let _ = cache::store_cover(&conn, &template_key, &template);
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
    if report_times { run_log.println(&format!("[TIMING] db inserts ({} articles): {:?}", pipeline_result.len(), t.elapsed())); }

    // ── Load from DB (respects limit) ────────────────────────────────────────

    let t = std::time::Instant::now();
    let all_articles = cache::load_articles(&conn, &cfg.url, cfg.limit)?;
    if report_times { run_log.println(&format!("[TIMING] db load ({} articles): {:?}", all_articles.len(), t.elapsed())); }

    if all_articles.is_empty() {
        run_log.println("No articles found.");
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
        cache::load_images(&conn, &sha1s)?
            .into_iter()
            .map(|img| (img.url_sha1.clone(), img))
            .collect()
    };

    // ── Build EPUB / KEPUB ────────────────────────────────────────────────────

    run_log.println(&format!("Building {} ({} articles)...",
        if cfg.kobo { "KEPUB" } else { "EPUB" }, all_articles.len()));

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
        run_log.println(&format!("[TIMING] epub build: {:?}", t.elapsed()));
        run_log.println(&format!("[TIMING] total: {:?}", t_start.elapsed()));
    }

    run_log.println(&format!("Written: {}", output_display));

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    println!("Feedbook - v{}", env!("CARGO_PKG_VERSION"));

    let args = Args::parse();

    let config_result = config::load_config(args.config.as_deref())?;

    let log_enabled = args.log.unwrap_or(false)
        || config_result.as_ref()
            .and_then(|(rc, _)| rc.defaults.as_ref()?.log)
            .unwrap_or(false);

    let log_file: Option<log::LogFile> = if log_enabled {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        let path = exe_dir.join("feedbook.log");
        match std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&path) {
            Ok(f) => Some(std::sync::Arc::new(std::sync::Mutex::new(std::io::BufWriter::new(f)))),
            Err(e) => {
                eprintln!("Warning: could not open log file {}: {e}", path.display());
                None
            }
        }
    } else {
        None
    };

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
                println!("No feeds to process.");
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

    cache::prune_images(&cache::open_db(&db_path)?)?;

    futures::stream::iter(feeds_to_run)
        .map(|cfg| {
            let client = client.clone();
            let log_file = log_file.clone();
            async move {
                let url = cfg.url.clone();
                if let Err(e) = run_feed(&cfg, &client, log_file).await {
                    eprintln!("{}: Error: {e}", feed_log_prefix(&url));
                }
            }
        })
        .buffer_unordered(5)
        .collect::<Vec<()>>()
        .await;

    Ok(())
}
