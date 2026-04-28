use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("HTTP error: {0}")]
    Http(#[from] wreq::Error),

    #[error("Feed parse error: {0}")]
    Feed(String),

    #[error("Database error: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("EPUB error: {0}")]
    Epub(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config file not found: {0}")]
    ConfigNotFound(String),

    #[error("Config error in {path}: {msg}")]
    Config { path: String, msg: String },

    #[error("Config parse error in {path}: {source}")]
    ConfigParse { path: String, #[source] source: toml::de::Error },

    #[error("No feed URL provided and no config file found")]
    NoUrl,

    #[error("{0}")]
    Other(String),
}
