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
            article_url TEXT    NOT NULL UNIQUE,
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
            data       BLOB    NOT NULL,
            created_at INTEGER NOT NULL
        );",
    )?;
    Ok(conn)
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
        "DELETE FROM covers WHERE created_at < (strftime('%s', 'now') - 2592000)",
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

pub fn get_cached_cover(conn: &Connection, key: &str) -> Result<Option<Vec<u8>>, AppError> {
    let mut stmt = conn.prepare("SELECT data FROM covers WHERE cache_key = ?1")?;
    let mut rows = stmt.query(params![key])?;
    if let Some(row) = rows.next()? {
        let data: Vec<u8> = row.get(0)?;
        return Ok(Some(data));
    }
    Ok(None)
}

pub fn store_cover(conn: &Connection, key: &str, data: &[u8]) -> Result<(), AppError> {
    let created_at = Utc::now().timestamp();
    conn.execute(
        "INSERT OR REPLACE INTO covers (cache_key, data, created_at) VALUES (?1, ?2, ?3)",
        params![key, data, created_at],
    )?;
    Ok(())
}
