# feedbook

Convert any RSS or Atom feed into a clean, readable EPUB file. Point it at a feed URL and get a properly formatted e-book with full article content, ready for your e-reader.

## Usage

```
feedbook --url <feed-url> [--limit <n>]
```

| Flag      | Description                                                    |
|-----------|----------------------------------------------------------------|
| `--url`   | URL of an RSS or Atom feed (required)                          |
| `--limit` | Maximum number of articles to include (optional, newest first) |

The output file is written to the current directory, named after the feed title (e.g. `hacker-news.epub`).

### Examples

```bash
# Full feed
feedbook --url https://example.com/feed.rss

# Latest 10 articles only
feedbook --url https://example.com/feed.rss --limit 10
```

### Caching

feedbook keeps a local SQLite cache at `%LOCALAPPDATA%\feedbook\feedbook.sql` (Windows) or `~/.local/share/feedbook/feedbook.sql` (Linux/macOS). On subsequent runs against the same feed, only new articles are fetched — already-downloaded articles are loaded from the cache. Re-generating an EPUB for a feed you've hit before is near-instant.

The cache is pruned automatically: a maximum of 500 articles are kept per feed, and any article older than 90 days is removed.

---

## Building from source

### Prerequisites

**1. Rust toolchain**

Install via [rustup](https://rustup.rs). The project uses the 2024 edition, so you need Rust 1.85 or later.

### Build

```bash
git clone <repo>
cd feedbook
cargo build --release
```

The binary lands at `target/release/feedbook` (or `feedbook.exe` on Windows).
