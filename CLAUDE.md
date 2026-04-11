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

## Windows build requirements

Three native dependencies must be configured via `.cargo/config.toml` (already committed):

1. **libxml2 via vcpkg** — installed as `libxml2:x64-windows-static-md` (statically linked). `VCPKG_ROOT` and `VCPKGRS_TRIPLET` point to `C:/CHDEV/vcpkg`.
2. **LLVM/Clang for bindgen** — LLVM 22+ at `C:/Program Files/LLVM`. `LIBCLANG_PATH` uses `force = true` to override the ESP Rust toolchain's wrong-architecture `libclang.dll`.
3. Do **not** install `libxml2:x64-windows` (the dynamic triplet) alongside `x64-windows-static-md` — the `libxml` crate's build script does a `vcpkg list libxml2` and picks the first result; if the 2.10.x dynamic package appears first, it incorrectly sets `cfg(libxml_older_than_2_12)` and causes a type mismatch compile error. Only `x64-windows-static-md` should be present.

If the release build fails with a `*const _xmlError` / `*mut _xmlError` mismatch, delete `target/release/build/libxml-*` and rebuild — the build script may be using a stale cached output from a prior bad state.

## Architecture

The pipeline in `main.rs` runs sequentially:

```
CLI args → reqwest::Client → SQLite open+prune → fetch feed → filter cached → scrape new → insert DB → load all from DB → build EPUB
```

All DB operations are **synchronous** on the main thread (`rusqlite::Connection` is `!Send`). Async is used only for HTTP: fetching the feed and scraping articles.

### Module responsibilities

- **`feed.rs`** — fetches raw feed bytes, parses with `feed-rs`, returns `(title, Vec<FeedItem>)`. `FeedItem` carries URL, title, author, date.
- **`scraper.rs`** — wraps `ArticleScraper` (not `Clone`) in an `Arc`, fans out over items using `futures::stream::buffer_unordered(5)`, prefers article_scraper metadata over feed metadata with fallback. Returns `Vec<ScrapedArticle>`.
- **`cache.rs`** — SQLite schema, open/prune/read/write. Pruning: 500-article per-feed cap + 90-day TTL, run on every startup before any fetching. `load_articles` is the authoritative source for what goes into the EPUB (including limit enforcement).
- **`epub.rs`** — wraps `epub-builder` with `ZipLibrary` backend. Each article becomes one `.xhtml` chapter. `derive_output_path` slugifies the feed title.
- **`error.rs`** — single `AppError` enum via `thiserror`.
- **`cli.rs`** — `clap` derive struct; `--url` (required) and `--limit` (optional).

### Key data flow detail

`--limit` is applied **twice**: once after fetching the feed (to avoid scraping more articles than needed), and once in `load_articles` (SQL `LIMIT`). This means limit applies to the final EPUB, but articles already in the cache from prior unlimited runs are still available.
