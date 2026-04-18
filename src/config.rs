use std::path::{Path, PathBuf};
use serde::Deserialize;
use crate::cli::Args;
use crate::error::AppError;

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(deny_unknown_fields)]
pub struct RawDefaults {
    pub outfolder:       Option<String>,
    pub dbpath:          Option<String>,
    pub limit:           Option<usize>,
    pub kobo:            Option<bool>,
    pub no_images:       Option<bool>,
    pub max_image_width: Option<u32>,
    pub force:           Option<bool>,
    pub stdout:          Option<bool>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(deny_unknown_fields)]
pub struct RawFeed {
    pub url:             String,
    pub name:            Option<String>,
    pub enabled:         Option<bool>,
    pub limit:           Option<usize>,
    pub kobo:            Option<bool>,
    pub no_images:       Option<bool>,
    pub max_image_width: Option<u32>,
    pub force:           Option<bool>,
    pub stdout:          Option<bool>,
    pub outfolder:       Option<String>,
    // dbpath intentionally absent — deny_unknown_fields rejects it with a clear error
}

impl RawFeed {
    pub fn ad_hoc(url: String) -> Self {
        Self {
            url,
            name: None,
            enabled: Some(true),
            limit: None,
            kobo: None,
            no_images: None,
            max_image_width: None,
            force: None,
            stdout: None,
            outfolder: None,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RawConfig {
    pub defaults: Option<RawDefaults>,
    #[serde(default)]
    pub feeds: Vec<RawFeed>,
}

pub struct ResolvedFeedConfig {
    pub url:             String,
    pub name:            Option<String>,
    pub limit:           Option<usize>,
    pub force:           bool,
    pub no_images:       bool,
    pub max_image_width: u32,
    pub dbpath:          Option<String>,
    pub stdout:          bool,
    pub kobo:            bool,
    pub outfolder:       Option<String>,
}

pub fn resolve_path(raw: &str, config_dir: &Path) -> PathBuf {
    if raw.starts_with("~/") || raw.starts_with("~\\") {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(&raw[2..])
    } else {
        let p = PathBuf::from(raw);
        if p.is_absolute() { p } else { config_dir.join(p) }
    }
}

pub fn merge(cli: &Args, defaults: &RawDefaults, feed: &RawFeed, config_dir: &Path) -> ResolvedFeedConfig {
    let resolve = |s: &str| resolve_path(s, config_dir).display().to_string();

    ResolvedFeedConfig {
        url:             feed.url.clone(),
        name:            feed.name.clone(),
        limit:           cli.limit.or(feed.limit).or(defaults.limit),
        force:           cli.force.or(feed.force).or(defaults.force).unwrap_or(false),
        no_images:       cli.no_images.or(feed.no_images).or(defaults.no_images).unwrap_or(false),
        max_image_width: cli.max_image_width.or(feed.max_image_width).or(defaults.max_image_width).unwrap_or(460),
        dbpath:          cli.dbpath.clone()
                             .or_else(|| defaults.dbpath.as_deref().map(&resolve)),
        stdout:          cli.stdout.or(feed.stdout).or(defaults.stdout).unwrap_or(false),
        kobo:            cli.kobo.or(feed.kobo).or(defaults.kobo).unwrap_or(false),
        outfolder:       cli.outfolder.as_deref().map(&resolve)
                             .or_else(|| feed.outfolder.as_deref().map(&resolve))
                             .or_else(|| defaults.outfolder.as_deref().map(&resolve)),
    }
}

pub fn load_config(cli_config_path: Option<&str>) -> Result<Option<(RawConfig, PathBuf)>, AppError> {
    let path = if let Some(p) = cli_config_path {
        let p = PathBuf::from(p);
        if !p.exists() {
            return Err(AppError::ConfigNotFound(p.display().to_string()));
        }
        p
    } else {
        match config_search_paths().into_iter().find(|p| p.exists()) {
            Some(p) => p,
            None => return Ok(None),
        }
    };

    let content = std::fs::read_to_string(&path).map_err(|e| AppError::Config {
        path: path.display().to_string(),
        msg: e.to_string(),
    })?;

    let raw: RawConfig = toml::from_str(&content).map_err(|e| AppError::ConfigParse {
        path: path.display().to_string(),
        source: e,
    })?;

    Ok(Some((raw, path)))
}

fn config_search_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            paths.push(dir.join("feedbook.toml"));
        }
    }

    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("feedbook.toml"));
    }

    if let Some(config_dir) = dirs::config_dir() {
        paths.push(config_dir.join("feedbook").join("feedbook.toml"));
    }

    paths
}
