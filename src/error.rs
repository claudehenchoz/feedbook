use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Feed parse error: {0}")]
    Feed(String),

    #[error("Scraper error: {0}")]
    Scraper(String),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("EPUB error: {0}")]
    Epub(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("URL parse error: {0}")]
    Url(#[from] url::ParseError),

    #[error("{0}")]
    Other(String),
}
