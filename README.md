# feedbook

Convert any RSS or Atom feed into a clean, readable EPUB file. Point it at a feed URL and get a properly formatted e-book with full article content, images included, ready for your e-reader.

## Usage

```
feedbook [--url <feed-url>] [--config <path>] [options]
```

| Flag                | Default                  | Description                                                              |
|---------------------|--------------------------|--------------------------------------------------------------------------|
| `--url`                | *(see below)*            | URL of an RSS or Atom feed                                               |
| `--config`             | *(auto-discovered)*      | Path to a `feedbook.toml` config file                                    |
| `--limit`              | *(all)*                  | Maximum number of articles to include (newest first)                     |
| `--outfolder`          | current directory        | Directory where the output file is written                               |
| `--dbpath`             | system local-data dir    | Path to the SQLite database file, or a directory (uses `feedbook.sql`)   |
| `--kobo`               | off                      | Produce a Kobo KEPUB (`.kepub.epub`) instead of a standard EPUB          |
| `--stdout`             | off                      | Print plain log lines instead of progress bars (for CI/CD)               |
| `--force`              | off                      | Re-fetch all articles, ignoring and overwriting the cache                |
| `--no-images`          | off                      | Disable image downloading and embedding                                  |
| `--max-image-width`    | `460`                    | Maximum image width in pixels                                            |
| `--content-selectors`  | *(Readability)*          | One or more CSS selectors whose matched elements form the article body   |
| `--remove-selectors`   | *(none)*                 | One or more CSS selectors whose matched elements are stripped first       |

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

# Custom CSS selectors (bypasses Readability for sites where it struggles)
feedbook --url https://example.com/feed.rss --content-selectors "article" ".post-body" --remove-selectors ".nav" ".sidebar"

# Process all feeds defined in a config file
feedbook

# Use an alternate config file
feedbook --config ~/kobo-feeds.toml
```

### Output file naming

The output filename is derived from the feed title: lowercased, non-alphanumeric characters replaced with hyphens. A feed titled "Hacker News" produces `hacker-news.epub` (or `hacker-news.kepub.epub` with `--kobo`). The `name` key in a config file overrides the feed's self-reported title.

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

### Custom content selectors

By default feedbook uses the [Readability](https://github.com/mozilla/readability) algorithm to extract the main article body from each page. For sites where this produces poor results, you can take over with explicit CSS selectors:

- **`--content-selectors`** — one or more selectors whose matched elements are concatenated to form the article body. When this flag is set, Readability is bypassed entirely.
- **`--remove-selectors`** — selectors for elements to strip out *before* extraction (navigation, banners, comment sections, etc.). Applied only when `--content-selectors` is also set.

If the content selectors match nothing on a given page, feedbook falls back to Readability automatically.

Because the custom extraction path does not produce metadata, title, author, and date are always taken from the feed.

```bash
feedbook --url https://example.com/feed.rss \
  --content-selectors "article.post" \
  --remove-selectors ".post-footer" ".related-posts"
```

Both flags accept multiple space-separated values on the command line. In a config file, they are TOML string arrays (see [Per-feed keys](#per-feed-keys)).

---

## Config file

For regular use with one or more feeds, create a `feedbook.toml` instead of typing flags every time.

### Discovery order

feedbook looks for `feedbook.toml` in:

1. The directory containing the feedbook binary
2. The current working directory
3. `~/.config/feedbook/feedbook.toml` (or `%APPDATA%\feedbook\` on Windows)

Pass `--config <path>` to use a specific file instead.

### Structure

```toml
# Global defaults — applied to every feed unless overridden.
[defaults]
outfolder       = "~/ebooks"
limit           = 50
kobo            = false
no_images       = false
max_image_width = 460

# One [[feeds]] entry per feed, processed in order.
[[feeds]]
url   = "https://news.ycombinator.com/rss"
name  = "Hacker News"   # overrides the feed's own title (filename + cover)
limit = 30
kobo  = true

[[feeds]]
url       = "https://example.com/feed.rss"
outfolder = "~/ebooks/tech"   # override a single setting for this feed

[[feeds]]
url     = "https://another.example.com/rss"
enabled = false               # skip without deleting the entry
```

### Precedence

CLI flags > per-feed config > `[defaults]` > built-in defaults.

When `--url` is given alongside a config file, only that feed is processed. If the URL matches a `[[feeds]]` entry its settings apply; otherwise it runs as a one-off with `[defaults]`.

### Per-feed keys

| Key             | Type    | Description                                                    |
|-----------------|---------|----------------------------------------------------------------|
| Key                  | Type           | Description                                                    |
|----------------------|----------------|----------------------------------------------------------------|
| `url`                | string         | *(required)* Feed URL                                          |
| `name`               | string         | Override the feed's self-reported title                        |
| `enabled`            | bool           | Set to `false` to skip this feed (default: `true`)             |
| `limit`              | integer        | Max articles                                                   |
| `kobo`               | bool           | Produce KEPUB                                                  |
| `no_images`          | bool           | Strip images                                                   |
| `max_image_width`    | integer        | Resize wider images to this pixel width                        |
| `force`              | bool           | Re-fetch even cached articles                                  |
| `stdout`             | bool           | Plain log output                                               |
| `outfolder`          | string         | Output directory (tilde-expanded, relative to config file)     |
| `content_selectors`  | array of strings | CSS selectors for the article body; bypasses Readability when set |
| `remove_selectors`   | array of strings | CSS selectors for elements to strip before extraction          |

`dbpath` is only valid in `[defaults]`, not per feed.

A copy of `feedbook.example.toml` with all options annotated is included in the repository.

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
