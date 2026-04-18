# feedbook

Convert any RSS or Atom feed into a clean, readable EPUB file. Point it at a feed URL and get a properly formatted e-book with full article content, images included, ready for your e-reader.

## Usage

```
feedbook --url <feed-url> [options]
```

| Flag                | Default                  | Description                                                              |
|---------------------|--------------------------|--------------------------------------------------------------------------|
| `--url`             | *(required)*             | URL of an RSS or Atom feed                                               |
| `--limit`           | *(all)*                  | Maximum number of articles to include (newest first)                     |
| `--outfolder`       | current directory        | Directory where the output file is written                               |
| `--dbpath`          | system local-data dir    | Path to the SQLite database file, or a directory (uses `feedbook.sql`)   |
| `--kobo`            | off                      | Produce a Kobo KEPUB (`.kepub.epub`) instead of a standard EPUB          |
| `--stdout`          | off                      | Print plain log lines instead of progress bars (for CI/CD)               |
| `--force`           | off                      | Re-fetch all articles, ignoring and overwriting the cache                |
| `--no-images`       | off                      | Disable image downloading and embedding                                  |
| `--max-image-width` | `460`                    | Maximum image width in pixels                                            |

### Examples

```bash
# Full feed, standard EPUB in the current directory
feedbook --url https://example.com/feed.rss

# Latest 10 articles only
feedbook --url https://example.com/feed.rss --limit 10

# Write to a specific output folder
feedbook --url https://example.com/feed.rss --outfolder ~/ebooks

# Kobo KEPUB with output folder (produces hacker-news.kepub.epub)
feedbook --url https://news.ycombinator.com/rss --kobo --outfolder /mnt/kobo

# CI/CD-friendly: flat log output, custom DB location
feedbook --url https://example.com/feed.rss --stdout --dbpath /data --outfolder /output

# Force re-fetch, skip images
feedbook --url https://example.com/feed.rss --force --no-images
```

### Output file naming

The output filename is derived from the feed title: lowercased, non-alphanumeric characters replaced with hyphens. A feed titled "Hacker News" produces `hacker-news.epub` (or `hacker-news.kepub.epub` with `--kobo`).

### Caching

feedbook keeps a local SQLite cache (default: `%LOCALAPPDATA%\feedbook\feedbook.sql` on Windows, `~/.local/share/feedbook/feedbook.sql` on Linux/macOS). On subsequent runs against the same feed, only new articles are fetched — already-downloaded articles and images are loaded from the cache, making re-runs near-instant.

Use `--dbpath .` to store the database in the current directory, or `--dbpath /some/dir` for any other location.

The cache is pruned automatically on every run: a maximum of 500 articles are kept per feed, and articles older than 90 days are removed.

### Kobo KEPUB

With `--kobo`, feedbook produces a `.kepub.epub` file instead of a standard `.epub`. This unlocks Kobo's enhanced reading experience (reading statistics, precise bookmarks, highlights) compared to sideloaded plain EPUBs. The file is still a valid EPUB 3 archive — Kobo's firmware detects the `.kepub.epub` extension and applies its extended rendering pipeline.

### Stdout / CI mode

`--stdout` suppresses the interactive progress bars and emits simple log lines to stderr instead:

```
Feed: Hacker News
Article: Show HN: My project
Article: Ask HN: Something interesting
Generating cover...
Cover ready
Building EPUB (2 articles)...
Written: hacker-news.epub
```

---

## Building from source

### Prerequisites

**Rust toolchain** — install via [rustup](https://rustup.rs). Requires Rust 1.85 or later (2024 edition).

### Build

```bash
git clone <repo>
cd feedbook
cargo build --release
```

The binary lands at `target/release/feedbook` (or `feedbook.exe` on Windows).

### Build for Kobo

```bash
# One-time setup (needs Docker or Podman)
cargo install cross --git https://github.com/cross-rs/cross

cross build --release --target armv7-unknown-linux-musleabihf
```
