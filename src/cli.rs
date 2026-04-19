use clap::Parser;

#[derive(Parser)]
#[command(name = "feedbook", about = "Generate EPUB from RSS/Atom feed")]
pub struct Args {
    /// URL of the RSS/Atom feed (optional when a config file is present)
    #[arg(long)]
    pub url: Option<String>,

    /// Path to a feedbook.toml config file
    #[arg(long)]
    pub config: Option<String>,

    /// Maximum number of articles to include
    #[arg(long)]
    pub limit: Option<usize>,

    /// Re-fetch all articles from the web, ignoring and overwriting the cache
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub force: Option<bool>,

    /// Disable image downloading and embedding
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub no_images: Option<bool>,

    /// Maximum image width in pixels
    #[arg(long)]
    pub max_image_width: Option<u32>,

    /// Path to the SQLite database file or directory (default: system local-data dir)
    #[arg(long)]
    pub dbpath: Option<String>,

    /// Print plain log lines instead of progress bars (for CI/CD)
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub stdout: Option<bool>,

    /// Produce Kobo KEPUB (.kepub.epub) instead of standard EPUB
    #[arg(long, num_args = 0..=1, default_missing_value = "true")]
    pub kobo: Option<bool>,

    /// Directory where the output EPUB/KEPUB should be written (default: current dir)
    #[arg(long)]
    pub outfolder: Option<String>,

    /// CSS selectors whose matching elements form the article body (bypasses Readability)
    #[arg(long, num_args = 1..)]
    pub content_selectors: Option<Vec<String>>,

    /// CSS selectors whose matching elements are stripped before extraction
    #[arg(long, num_args = 1..)]
    pub remove_selectors: Option<Vec<String>>,
}
