/// Extracts the second-level domain from a URL as a short display title.
/// "https://www.inoreader.com/..." → "inoreader"
pub fn extract_domain_title(feed_url: &str) -> String {
    let host = match url::Url::parse(feed_url).ok().and_then(|u| u.host_str().map(str::to_owned)) {
        Some(h) => h,
        None => return "feed".to_string(),
    };
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() >= 2 {
        parts[parts.len() - 2].to_string()
    } else {
        host
    }
}
