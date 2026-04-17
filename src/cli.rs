use clap::Parser;

#[derive(Parser)]
#[command(name = "feedbook", about = "Generate EPUB from RSS/Atom feed")]
pub struct Args {
    /// URL of the RSS/Atom feed
    #[arg(long)]
    pub url: String,

    /// Maximum number of articles to include
    #[arg(long)]
    pub limit: Option<usize>,

    /// Re-fetch all articles from the web, ignoring and overwriting the cache
    #[arg(long)]
    pub force: bool,

    /// Disable image downloading and embedding
    #[arg(long)]
    pub no_images: bool,

    /// Maximum image width in pixels
    #[arg(long, default_value_t = 460)]
    pub max_image_width: u32,

    /// Path to the SQLite database file or directory (default: system local-data dir)
    #[arg(long)]
    pub dbpath: Option<String>,

    /// Print plain log lines instead of progress bars (for CI/CD)
    #[arg(long)]
    pub stdout: bool,

    /// Produce Kobo KEPUB (.kepub.epub) instead of standard EPUB
    #[arg(long)]
    pub kobo: bool,

    /// Directory where the output EPUB/KEPUB should be written (default: current dir)
    #[arg(long)]
    pub outfolder: Option<String>,
}
