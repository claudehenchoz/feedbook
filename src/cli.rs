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
}
