use std::collections::HashSet;
use std::path::PathBuf;
use chrono::Utc;
use rusqlite::{Connection, params};
use crate::error::AppError;
use crate::images::ProcessedImage;
use crate::scraper::ScrapedArticle;

pub fn db_path() -> Result<PathBuf, AppError> {
    let mut path = dirs::data_local_dir()
        .ok_or_else(|| AppError::Other("Could not find local data directory".to_string()))?;
    path.push("feedbook");
    std::fs::create_dir_all(&path)?;
    path.push("feedbook.sql");
    Ok(path)
}

pub fn open_db(path: &PathBuf) -> Result<Connection, AppError> {
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
        PRAGMA synchronous=NORMAL;
        CREATE TABLE IF NOT EXISTS articles (
            id          INTEGER PRIMARY KEY AUTOINCREMENT,
            feed_url    TEXT    NOT NULL,
            article_url TEXT    NOT NULL,
            title       TEXT,
            author      TEXT,
            date_iso    TEXT,
            html        TEXT,
            fetched_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_feed_url   ON articles(feed_url);
        CREATE INDEX IF NOT EXISTS idx_fetched_at ON articles(fetched_at);
        CREATE TABLE IF NOT EXISTS images (
            url_sha1   TEXT    PRIMARY KEY,
            orig_url   TEXT    NOT NULL,
            filename   TEXT    NOT NULL,
            data       BLOB    NOT NULL,
            fetched_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_images_fetched_at ON images(fetched_at);
        CREATE TABLE IF NOT EXISTS covers (
            cache_key  TEXT    PRIMARY KEY,
            width      INTEGER NOT NULL DEFAULT 0,
            height     INTEGER NOT NULL DEFAULT 0,
            data       BLOB    NOT NULL,
            created_at INTEGER NOT NULL
        );",
    )?;
    // Migrations for existing installs — succeed on upgrade, ignored on fresh create
    let _ = conn.execute_batch("ALTER TABLE covers ADD COLUMN width  INTEGER NOT NULL DEFAULT 0");
    let _ = conn.execute_batch("ALTER TABLE covers ADD COLUMN height INTEGER NOT NULL DEFAULT 0");
    Ok(conn)
}

pub struct CachedTemplate {
    pub width:  u32,
    pub height: u32,
    pub data:   Vec<u8>,
}

pub fn prune(conn: &Connection, feed_url: &str) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM articles
         WHERE feed_url = ?1
           AND id NOT IN (
               SELECT id FROM articles
               WHERE feed_url = ?1
               ORDER BY fetched_at DESC
               LIMIT 500
           )",
        params![feed_url],
    )?;
    conn.execute(
        "DELETE FROM articles WHERE fetched_at < (strftime('%s', 'now') - 7776000)",
        [],
    )?;
    conn.execute(
        "DELETE FROM covers WHERE created_at < (strftime('%s', 'now') - 7776000)",
        [],
    )?;
    Ok(())
}

pub fn get_cached_urls(conn: &Connection, feed_url: &str) -> Result<HashSet<String>, AppError> {
    let mut stmt =
        conn.prepare("SELECT article_url FROM articles WHERE feed_url = ?1")?;
    let urls = stmt
        .query_map(params![feed_url], |row| row.get(0))?
        .collect::<Result<HashSet<String>, _>>()?;
    Ok(urls)
}

pub fn insert_article(
    conn: &Connection,
    feed_url: &str,
    article: &ScrapedArticle,
) -> Result<(), AppError> {
    let fetched_at = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO articles
             (feed_url, article_url, title, author, date_iso, html, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            feed_url,
            article.url,
            article.title,
            article.author,
            article.date.map(|d| d.to_rfc3339()),
            article.html,
            fetched_at,
        ],
    )?;
    Ok(())
}

pub fn load_articles(
    conn: &Connection,
    feed_url: &str,
    limit: Option<usize>,
) -> Result<Vec<ScrapedArticle>, AppError> {
    let sql = match limit {
        Some(n) => format!(
            "SELECT article_url, title, author, date_iso, html \
             FROM articles WHERE feed_url = ?1 \
             ORDER BY COALESCE(date_iso, '') DESC LIMIT {}",
            n
        ),
        None => "SELECT article_url, title, author, date_iso, html \
                 FROM articles WHERE feed_url = ?1 \
                 ORDER BY COALESCE(date_iso, '') DESC"
            .to_string(),
    };

    let mut stmt = conn.prepare(&sql)?;
    let articles = stmt
        .query_map(params![feed_url], |row| {
            let url: String = row.get(0)?;
            let title: Option<String> = row.get(1)?;
            let author: Option<String> = row.get(2)?;
            let date_iso: Option<String> = row.get(3)?;
            let html: Option<String> = row.get(4)?;
            let date = date_iso.and_then(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&chrono::Utc))
            });
            Ok(ScrapedArticle { url, title, author, date, html })
        })?
        .collect::<Result<Vec<_>, _>>()?;

    Ok(articles)
}

pub fn prune_images(conn: &Connection) -> Result<(), AppError> {
    conn.execute(
        "DELETE FROM images WHERE fetched_at < (strftime('%s', 'now') - 7776000)",
        [],
    )?;
    Ok(())
}

pub fn get_cached_image_sha1s(conn: &Connection) -> Result<HashSet<String>, AppError> {
    let mut stmt = conn.prepare("SELECT url_sha1 FROM images")?;
    let sha1s = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<HashSet<String>, _>>()?;
    Ok(sha1s)
}

pub fn insert_image(conn: &Connection, img: &ProcessedImage) -> Result<(), AppError> {
    let fetched_at = Utc::now().timestamp();
    conn.execute(
        "INSERT OR IGNORE INTO images (url_sha1, orig_url, filename, data, fetched_at)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![img.url_sha1, img.original_url, img.filename, img.data, fetched_at],
    )?;
    Ok(())
}

pub fn load_images(conn: &Connection, sha1s: &[String]) -> Result<Vec<ProcessedImage>, AppError> {
    if sha1s.is_empty() {
        return Ok(vec![]);
    }
    let placeholders = sha1s.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!(
        "SELECT url_sha1, orig_url, filename, data FROM images WHERE url_sha1 IN ({})",
        placeholders
    );
    let mut stmt = conn.prepare(&sql)?;
    let params: Vec<&dyn rusqlite::ToSql> =
        sha1s.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
    let images = stmt
        .query_map(params.as_slice(), |row| {
            Ok(ProcessedImage {
                url_sha1:     row.get(0)?,
                original_url: row.get(1)?,
                filename:     row.get(2)?,
                data:         row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(images)
}

pub fn get_cached_cover(conn: &Connection, key: &str) -> Result<Option<CachedTemplate>, AppError> {
    let mut stmt = conn.prepare("SELECT width, height, data FROM covers WHERE cache_key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    if let Some(row) = rows.next()? {
        let width:  u32     = row.get(0)?;
        let height: u32     = row.get(1)?;
        let data:   Vec<u8> = row.get(2)?;
        return Ok(Some(CachedTemplate { width, height, data }));
    }
    Ok(None)
}

pub fn store_cover(conn: &Connection, key: &str, width: u32, height: u32, data: &[u8]) -> Result<(), AppError> {
    let created_at = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO covers (cache_key, width, height, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![key, width, height, data, created_at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem_db() -> Connection {
        open_db(&PathBuf::from(":memory:")).unwrap()
    }

    fn make_article(url: &str) -> ScrapedArticle {
        ScrapedArticle {
            url: url.to_string(),
            title: Some(format!("Title for {}", url)),
            author: Some("Author".to_string()),
            date: Some(chrono::DateTime::parse_from_rfc3339("2024-01-15T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc)),
            html: Some("<p>body</p>".to_string()),
        }
    }

    fn make_image(sha1: &str, url: &str) -> ProcessedImage {
        ProcessedImage {
            url_sha1: sha1.to_string(),
            original_url: url.to_string(),
            filename: format!("images/{}.jpeg", sha1),
            data: vec![0xFFu8, 0xD8, 0xFF], // minimal JPEG header bytes
        }
    }

    #[test]
    fn open_db_creates_tables() {
        let conn = mem_db();
        // Verify all three tables exist by counting rows (would fail if tables are missing)
        let articles: i64 = conn
            .query_row("SELECT COUNT(*) FROM articles", [], |r| r.get(0))
            .unwrap();
        let images: i64 = conn
            .query_row("SELECT COUNT(*) FROM images", [], |r| r.get(0))
            .unwrap();
        let covers: i64 = conn
            .query_row("SELECT COUNT(*) FROM covers", [], |r| r.get(0))
            .unwrap();
        assert_eq!(articles, 0);
        assert_eq!(images, 0);
        assert_eq!(covers, 0);
    }

    #[test]
    fn insert_and_get_cached_urls() {
        let conn = mem_db();
        let feed = "https://example.com/feed";
        let article = make_article("https://example.com/a1");
        insert_article(&conn, feed, &article).unwrap();
        let urls = get_cached_urls(&conn, feed).unwrap();
        assert!(urls.contains("https://example.com/a1"));
        assert_eq!(urls.len(), 1);
    }

    #[test]
    fn get_cached_urls_only_for_feed() {
        let conn = mem_db();
        insert_article(&conn, "https://feed-a.com/", &make_article("https://feed-a.com/a1")).unwrap();
        insert_article(&conn, "https://feed-b.com/", &make_article("https://feed-b.com/b1")).unwrap();
        let urls_a = get_cached_urls(&conn, "https://feed-a.com/").unwrap();
        assert!(urls_a.contains("https://feed-a.com/a1"));
        assert!(!urls_a.contains("https://feed-b.com/b1"));
    }

    #[test]
    fn load_articles_empty() {
        let conn = mem_db();
        let articles = load_articles(&conn, "https://example.com/feed", None).unwrap();
        assert!(articles.is_empty());
    }

    #[test]
    fn load_articles_with_limit() {
        let conn = mem_db();
        let feed = "https://example.com/feed";
        for i in 0..5 {
            insert_article(&conn, feed, &make_article(&format!("https://example.com/{}", i))).unwrap();
        }
        let articles = load_articles(&conn, feed, Some(3)).unwrap();
        assert_eq!(articles.len(), 3);
    }

    #[test]
    fn load_articles_ordered_by_date_desc() {
        let conn = mem_db();
        let feed = "https://example.com/feed";
        let mut older = make_article("https://example.com/older");
        older.date = Some(chrono::DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
            .unwrap().with_timezone(&Utc));
        let mut newer = make_article("https://example.com/newer");
        newer.date = Some(chrono::DateTime::parse_from_rfc3339("2024-06-01T00:00:00Z")
            .unwrap().with_timezone(&Utc));
        insert_article(&conn, feed, &older).unwrap();
        insert_article(&conn, feed, &newer).unwrap();
        let articles = load_articles(&conn, feed, None).unwrap();
        assert_eq!(articles[0].url, "https://example.com/newer");
        assert_eq!(articles[1].url, "https://example.com/older");
    }

    #[test]
    fn insert_or_replace_updates_title() {
        let conn = mem_db();
        let feed = "https://example.com/feed";
        let mut article = make_article("https://example.com/a1");
        article.title = Some("Original".to_string());
        insert_article(&conn, feed, &article).unwrap();
        article.title = Some("Updated".to_string());
        insert_article(&conn, feed, &article).unwrap();
        let articles = load_articles(&conn, feed, None).unwrap();
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0].title.as_deref(), Some("Updated"));
    }

    #[test]
    fn prune_by_count_keeps_500() {
        let conn = mem_db();
        let feed = "https://example.com/feed";
        for i in 0..505usize {
            insert_article(&conn, feed, &make_article(&format!("https://example.com/{}", i))).unwrap();
        }
        prune(&conn, feed).unwrap();
        let articles = load_articles(&conn, feed, None).unwrap();
        assert_eq!(articles.len(), 500);
    }

    #[test]
    fn prune_by_age_removes_old() {
        let conn = mem_db();
        let feed = "https://example.com/feed";
        let url = "https://example.com/old";
        insert_article(&conn, feed, &make_article(url)).unwrap();
        // Backdate the article so it appears >90 days old
        conn.execute("UPDATE articles SET fetched_at = 0 WHERE article_url = ?1", params![url]).unwrap();
        prune(&conn, feed).unwrap();
        let urls = get_cached_urls(&conn, feed).unwrap();
        assert!(!urls.contains(url));
    }

    #[test]
    fn prune_covers_by_age() {
        let conn = mem_db();
        store_cover(&conn, "old-cover", 1, 1, b"\0\0\0\0").unwrap();
        conn.execute("UPDATE covers SET created_at = 0 WHERE cache_key = 'old-cover'", []).unwrap();
        prune(&conn, "https://example.com/feed").unwrap();
        let result = get_cached_cover(&conn, "old-cover").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn insert_and_load_images() {
        let conn = mem_db();
        let img1 = make_image("aaa111", "https://example.com/img1.jpg");
        let img2 = make_image("bbb222", "https://example.com/img2.jpg");
        insert_image(&conn, &img1).unwrap();
        insert_image(&conn, &img2).unwrap();
        let sha1s = vec!["aaa111".to_string(), "bbb222".to_string()];
        let loaded = load_images(&conn, &sha1s).unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn insert_image_ignores_duplicate() {
        let conn = mem_db();
        let img = make_image("abc123", "https://example.com/img.jpg");
        insert_image(&conn, &img).unwrap();
        insert_image(&conn, &img).unwrap(); // second insert should be no-op
        let sha1s = vec!["abc123".to_string()];
        let loaded = load_images(&conn, &sha1s).unwrap();
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn prune_images_by_age() {
        let conn = mem_db();
        let img = make_image("old123", "https://example.com/old.jpg");
        insert_image(&conn, &img).unwrap();
        conn.execute("UPDATE images SET fetched_at = 0 WHERE url_sha1 = 'old123'", []).unwrap();
        prune_images(&conn).unwrap();
        let sha1s = get_cached_image_sha1s(&conn).unwrap();
        assert!(!sha1s.contains("old123"));
    }

    #[test]
    fn get_cached_image_sha1s_returns_all() {
        let conn = mem_db();
        insert_image(&conn, &make_image("sha1a", "https://example.com/a.jpg")).unwrap();
        insert_image(&conn, &make_image("sha1b", "https://example.com/b.jpg")).unwrap();
        let sha1s = get_cached_image_sha1s(&conn).unwrap();
        assert!(sha1s.contains("sha1a"));
        assert!(sha1s.contains("sha1b"));
        assert_eq!(sha1s.len(), 2);
    }

    #[test]
    fn store_and_get_cover() {
        let conn = mem_db();
        let data = vec![0u8; 100 * 200 * 4];
        store_cover(&conn, "my-cover", 100, 200, &data).unwrap();
        let result = get_cached_cover(&conn, "my-cover").unwrap().unwrap();
        assert_eq!(result.width, 100);
        assert_eq!(result.height, 200);
        assert_eq!(result.data.len(), 100 * 200 * 4);
    }

    #[test]
    fn get_cover_missing_returns_none() {
        let conn = mem_db();
        let result = get_cached_cover(&conn, "nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_images_empty_sha1s_returns_empty() {
        let conn = mem_db();
        let result = load_images(&conn, &[]).unwrap();
        assert!(result.is_empty());
    }
}
