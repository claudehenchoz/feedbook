# feedbook

Convert any RSS or Atom feed into a clean, readable EPUB file. Point it at a feed URL and get a properly formatted e-book with full article content, ready for your e-reader.

## Usage

```
feedbook --url <feed-url> [--limit <n>]
```

| Flag | Description |
|---|---|
| `--url` | URL of an RSS or Atom feed (required) |
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

**2. libxml2** (required by the article extraction library)

feedbook depends on [`article_scraper`](https://crates.io/crates/article_scraper), which uses libxml2 under the hood.

- **Linux:** install the development package via your package manager:
  ```bash
  # Debian/Ubuntu
  sudo apt install libxml2-dev

  # Fedora/RHEL
  sudo dnf install libxml2-devel
  ```
- **macOS:** already available via Xcode Command Line Tools; or `brew install libxml2`.
- **Windows (MSVC):** install via [vcpkg](https://github.com/microsoft/vcpkg):
  ```
  vcpkg install libxml2:x64-windows-static-md
  ```

**3. LLVM/Clang** (Windows only — needed by bindgen to generate libxml2 bindings)

On Windows, install [LLVM](https://releases.llvm.org/) 16 or later. The easiest way:
```
winget install LLVM.LLVM
```

### Build

```bash
git clone <repo>
cd feedbook
cargo build --release
```

The binary lands at `target/release/feedbook` (or `feedbook.exe` on Windows).

### Windows: `.cargo/config.toml`

On Windows MSVC you need to tell the build where vcpkg and LLVM live. Create `.cargo/config.toml` in the project root (or edit the one already committed):

```toml
[env]
VCPKG_ROOT    = "C:/path/to/vcpkg"
VCPKGRS_TRIPLET = "x64-windows-static-md"
LIBCLANG_PATH = { value = "C:/Program Files/LLVM/bin", force = true }
```

`force = true` on `LIBCLANG_PATH` is important if you have other Rust toolchains installed (e.g. the ESP-IDF toolchain ships its own clang that targets the wrong architecture and would otherwise take precedence).
