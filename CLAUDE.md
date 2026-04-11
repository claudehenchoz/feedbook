# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build
cargo build
cargo build --release

# Run
cargo run -- --url <feed-url> [--limit <n>]

# Check (faster than build, no codegen)
cargo check
```

There are no tests at this time.

## Architecture

The pipeline in `main.rs` runs sequentially:

```
CLI args → reqwest::Client → SQLite open+prune → fetch feed → filter cached → scrape new → insert DB → load all from DB → build EPUB
```

All DB operations are **synchronous** on the main thread (`rusqlite::Connection` is `!Send`). Async is used only for HTTP: fetching the feed and scraping articles.

### Module responsibilities

- **`feed.rs`** — fetches raw feed bytes, parses with `feed-rs`, returns `(title, Vec<FeedItem>)`. `FeedItem` carries URL, title, author, date.
- **`scraper.rs`** — fetches raw HTML per article via reqwest, runs `dom_smoothie::Readability` parsing in `tokio::task::spawn_blocking` (synchronous, `!Send`), fans out using `buffer_unordered(5)`, prefers dom_smoothie metadata over feed metadata with fallback. Returns `Vec<ScrapedArticle>`.
- **`cache.rs`** — SQLite schema, open/prune/read/write. Pruning: 500-article per-feed cap + 90-day TTL, run on every startup before any fetching. `load_articles` is the authoritative source for what goes into the EPUB (including limit enforcement).
- **`epub.rs`** — wraps `epub-builder` with `ZipLibrary` backend. Each article becomes one `.xhtml` chapter. `derive_output_path` slugifies the feed title.
- **`error.rs`** — single `AppError` enum via `thiserror`.
- **`cli.rs`** — `clap` derive struct; `--url` (required) and `--limit` (optional).

### Key data flow detail

`--limit` is applied **twice**: once after fetching the feed (to avoid scraping more articles than needed), and once in `load_articles` (SQL `LIMIT`). This means limit applies to the final EPUB, but articles already in the cache from prior unlimited runs are still available.
