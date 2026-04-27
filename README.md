# <img src="assets/feedbook.svg" alt="Feedbook icon" width="32"> Feedbook

Turn any RSS or Atom feed into a clean, readable EPUB. Point it at a feed URL and get a properly formatted e-book with full article content and images, ready for your e-reader.

- **On a computer?** Keep reading.
- **On a Kobo?** See [Running Feedbook on Kobo](README.kobo.md).
- **On a Kindle?** See [Running Feedbook on Kindle](README.kindle.md).

## Quickstart

### 1. Install

Grab the zip for your OS from the [Releases page](../../releases):

| OS                | Zip                                   |
|-------------------|---------------------------------------|
| Windows (x86_64)  | `feedbook-windows-x86_64-*.zip`       |
| macOS (universal) | `feedbook-macos-universal-*.zip`      |
| Linux (x86_64)    | `feedbook-linux-x86_64-*.zip`         |

Unpack it anywhere. You'll find two files: the `feedbook` binary and a starter `feedbook.toml`.

On macOS and Linux you may need to mark the binary executable:

```bash
chmod +x feedbook
```

On macOS the first run is blocked by Gatekeeper. Right-click the binary in Finder, pick **Open**, and confirm — you only need to do this once.

### 2. First run

From the folder where you unpacked the zip:

```bash
./feedbook --url https://news.ycombinator.com/rss --limit 10
```

(On Windows: `feedbook.exe --url ... --limit 10`.)

Feedbook prints a line per event as it fetches the feed, scrapes article pages, downloads images, and builds the EPUB — each line is prefixed with the feed's domain:

```
news.ycombinator.com: Feed: Hacker News
news.ycombinator.com: Article: Show HN: My project
news.ycombinator.com: Article: Ask HN: Something interesting
news.ycombinator.com: Cover ready
news.ycombinator.com: Building EPUB (2 articles)...
news.ycombinator.com: Written: hacker-news.epub
```

When it finishes, `hacker-news.epub` is in the current directory. Copy it to your e-reader and open it.

That's it. Everything below is optional.

## Using a config file

Typing `--url` every time gets old once you have more than one feed. Drop a `feedbook.toml` next to the binary (the one shipped in the zip is a starting point) and just run `./feedbook` — it'll process every feed listed in the file.

```toml
[defaults]
outfolder       = "~/ebooks"
limit           = 50
max_image_width = 460

[[feeds]]
url   = "https://news.ycombinator.com/rss"
name  = "Hacker News"
limit = 30

[[feeds]]
url = "https://example.com/feed.rss"

[[feeds]]
url     = "https://another.example.com/rss"
enabled = false                # skip without deleting the entry
```

Feedbook searches for `feedbook.toml` in, in order:

1. The directory containing the `feedbook` binary
2. The current working directory
3. `~/.config/feedbook/feedbook.toml` (`%APPDATA%\feedbook\` on Windows)

Pass `--config <path>` to use a specific file instead.

**Precedence.** CLI flags beat per-feed keys, which beat `[defaults]`, which beats the built-in defaults.

**Single-feed mode with a config.** Passing `--url` alongside a config file processes only that feed. If the URL matches a `[[feeds]]` entry its settings apply; otherwise it runs as a one-off using `[defaults]`.

A fully-annotated `feedbook.example.toml` is included in the repository.

### Config keys

All keys are optional except `url` on each feed.

**`[defaults]` only** — `dbpath`, `log`.

**`[defaults]` or per-feed** — every key in the table below.

| Key                 | Type             | Default          | Description                                                         |
|---------------------|------------------|------------------|---------------------------------------------------------------------|
| `url`               | string           | —                | *(required, per-feed only)* Feed URL                                |
| `name`              | string           | feed's own title | Override the feed's self-reported title (affects filename + cover)  |
| `enabled`           | bool             | `true`           | Set to `false` to skip this feed                                    |
| `limit`             | integer          | all              | Max articles (newest first)                                         |
| `outfolder`         | string           | current dir      | Output directory (tilde-expanded; relative paths resolve to config) |
| `dbpath`            | string           | system data dir  | SQLite cache path; a directory gets `feedbook.sql` appended         |
| `kobo`              | bool             | `false`          | Produce `.kepub.epub` for Kobo readers                              |
| `no_images`         | bool             | `false`          | Strip images entirely                                               |
| `max_image_width`   | integer          | `460`            | Resize wider images to this pixel width                             |
| `force`             | bool             | `false`          | Re-fetch articles already in the cache                              |
| `content_selectors` | array of strings | —                | CSS selectors for the article body; bypasses Readability when set   |
| `remove_selectors`  | array of strings | —                | CSS selectors for elements to strip before extraction               |
| `report_times`      | bool             | `false`          | Print `[TIMING]` lines for each pipeline stage                      |
| `log`               | bool             | `false`          | Write a timestamped `feedbook.log` next to the binary every run     |

## Command-line flags

Every config key has a matching flag. CLI flags override everything.

| Flag                  | Description                                                              |
|-----------------------|--------------------------------------------------------------------------|
| `--url`               | Feed URL (optional when a config file is present)                        |
| `--config`            | Path to a `feedbook.toml`                                                |
| `--limit`             | Max articles                                                             |
| `--outfolder`         | Output directory                                                         |
| `--dbpath`            | SQLite cache path (file or directory)                                    |
| `--kobo`              | Produce KEPUB                                                            |
| `--no-images`         | Strip images                                                             |
| `--max-image-width`   | Max image width in pixels                                                |
| `--force`             | Re-fetch cached articles                                                 |
| `--content-selectors` | Space-separated list of selectors                                        |
| `--remove-selectors`  | Space-separated list of selectors                                        |
| `--report-times`      | Print timing info per stage                                              |
| `--log`               | Write `feedbook.log` next to the binary                                  |

### Examples

```bash
# Full feed, standard EPUB in the current directory
feedbook --url https://example.com/feed.rss

# Latest 10 articles, written to ~/ebooks
feedbook --url https://example.com/feed.rss --limit 10 --outfolder ~/ebooks

# Kobo KEPUB
feedbook --url https://news.ycombinator.com/rss --kobo

# CI/CD: pinned DB location and output folder
feedbook --url https://example.com/feed.rss --dbpath /data --outfolder /output

# Force re-fetch, skip images
feedbook --url https://example.com/feed.rss --force --no-images

# Custom extraction for a site where Readability struggles
feedbook --url https://example.com/feed.rss \
  --content-selectors "article.post" ".entry-content" \
  --remove-selectors ".post-footer" ".related-posts"

# Process every feed in feedbook.toml
feedbook
```

## How it works

**Output filenames** are derived from the feed title: lowercased, non-alphanumerics replaced with hyphens. "Hacker News" → `hacker-news.epub` (or `hacker-news.kepub.epub` with `--kobo`). The `name` key overrides the feed's self-reported title.

**Caching.** Feedbook keeps a local SQLite cache (default: `~/.local/share/feedbook/feedbook.sql` on Linux/macOS, `%LOCALAPPDATA%\feedbook\feedbook.sql` on Windows). Subsequent runs against the same feed only fetch new articles; cached articles and images load instantly. The cache auto-prunes on every run: 500 articles max per feed, 90-day TTL. Use `--dbpath .` to keep the cache next to the binary, or `--dbpath /some/dir` for a custom location.

**Kobo KEPUB.** With `--kobo`, Feedbook produces a `.kepub.epub` file. It's still a valid EPUB 3 archive — Kobo's firmware detects the extension and applies its extended rendering pipeline, unlocking reading statistics, precise bookmarks, and highlights that aren't available for plain sideloaded EPUBs.

**Custom content selectors.** By default Feedbook uses the [Readability](https://github.com/mozilla/readability) algorithm to extract article bodies. For sites where it produces poor results, `content_selectors` takes over: every matching element becomes part of the article body, and Readability is bypassed. `remove_selectors` strips noise (nav, banners, comments) *before* extraction — it only applies when `content_selectors` is also set. If the content selectors match nothing on a given page, Feedbook falls back to Readability automatically. Because the custom path produces no metadata, title, author, and date come from the feed.

**Output.** Feedbook always emits one log line per event to stdout, prefixed with the feed's hostname so lines stay readable when processing multiple feeds:

```
hnrss.org: Feed: Hacker News: Newest
hnrss.org: Cover template cached
hnrss.org: Cover ready
hnrss.org: Article: How LLMs Work — A Visual Deep Dive
hnrss.org: Building EPUB (10 articles)...
hnrss.org: Written: hacker-news-newest.epub
```

**Log file** (`--log`, or `log = true` in `[defaults]`) mirrors the same events to `feedbook.log` next to the binary, with timestamps. The file is truncated on each run.

## Building from source

Requires Rust 1.85 or later (2024 edition) — install via [rustup](https://rustup.rs).

```bash
git clone <repo>
cd feedbook
cargo build --release
cargo test
```

(Building on Windows seems to need this before compiling: `$env:BINDGEN_EXTRA_CLANG_ARGS = "--target=x86_64-pc-windows-msvc -fms-compatibility -fms-extensions -fdeclspec"`)

The binary lands at `target/release/feedbook`.

### Cross-compiling for Kobo / Kindle

Kobo and older Kindles (Paperwhite 3 and older) need `armv7-unknown-linux-musleabihf`; newer Kindles (Paperwhite 4+) need `aarch64-unknown-linux-musl`. Use [`cross`](https://github.com/cross-rs/cross) (needs Docker or Podman):

```bash
cargo install cross --git https://github.com/cross-rs/cross

cross build --release --target armv7-unknown-linux-musleabihf   # Kobo, old Kindle
cross build --release --target aarch64-unknown-linux-musl       # new Kindle
```

Pre-built device zips are available on the [Releases page](../../releases).
