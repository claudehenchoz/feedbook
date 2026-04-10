use std::collections::HashSet;
use std::path::PathBuf;
use chrono::Utc;
use rusqlite::{Connection, params};
use crate::error::AppError;
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
        "CREATE TABLE IF NOT EXISTS articles (
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
        CREATE INDEX IF NOT EXISTS idx_fetched_at ON articles(fetched_at);",
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
