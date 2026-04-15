use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const THROTTLE_MS: u64 = 100;

/// Shared map of hostname → time of the last request to that host.
pub type HostTimes = Arc<Mutex<HashMap<String, Instant>>>;

pub fn new_host_times() -> HostTimes {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Like `client.get(url).send()`, but enforces a per-host minimum gap of
/// THROTTLE_MS between successive requests to the same host.
///
/// The lock is never held across an `.await` point.
pub async fn throttled_get(
    client: &reqwest::Client,
    url: &str,
    times: &HostTimes,
) -> reqwest::Result<reqwest::Response> {
    if let Ok(parsed) = url::Url::parse(url) {
        if let Some(host) = parsed.host_str().map(str::to_owned) {
            let min_gap = Duration::from_millis(THROTTLE_MS);

            // Phase 1: check if we need to sleep (drop lock before awaiting).
            let sleep_dur = {
                let map = times.lock().await;
                map.get(&host).and_then(|last| {
                    let elapsed = last.elapsed();
                    if elapsed < min_gap { Some(min_gap - elapsed) } else { None }
                })
            };

            if let Some(d) = sleep_dur {
                tokio::time::sleep(d).await;
            }

            // Phase 2: record that a request is going out now (drop lock before request).
            {
                let mut map = times.lock().await;
                map.insert(host, Instant::now());
            }
        }
    }

    client.get(url).send().await
}
