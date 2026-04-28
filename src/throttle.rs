use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const THROTTLE_MS: u64 = 200;

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
    client: &wreq::Client,
    url: &str,
    times: &HostTimes,
) -> wreq::Result<wreq::Response> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;
    use httpmock::prelude::*;

    fn test_client() -> wreq::Client {
        wreq::Client::new()
    }

    #[tokio::test]
    async fn throttled_get_returns_ok_response() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/resource");
            then.status(200).body("ok");
        });
        let client = test_client();
        let times = new_host_times();
        let resp = throttled_get(&client, &server.url("/resource"), &times)
            .await
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
    }

    #[tokio::test]
    async fn throttled_get_enforces_200ms_gap_same_host() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/resource");
            then.status(200).body("ok");
        });
        let client = test_client();
        let times = new_host_times();
        let url = server.url("/resource");

        let start = Instant::now();
        throttled_get(&client, &url, &times).await.unwrap();
        throttled_get(&client, &url, &times).await.unwrap();
        let elapsed = start.elapsed();

        // Two requests to the same host must be at least THROTTLE_MS apart.
        assert!(
            elapsed.as_millis() >= THROTTLE_MS as u128,
            "elapsed {}ms < {}ms throttle",
            elapsed.as_millis(),
            THROTTLE_MS
        );
    }

    #[tokio::test]
    async fn throttled_get_populates_host_times_map() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(GET).path("/resource");
            then.status(200).body("ok");
        });
        let client = test_client();
        let times = new_host_times();
        assert!(times.lock().await.is_empty());
        throttled_get(&client, &server.url("/resource"), &times).await.unwrap();
        assert!(!times.lock().await.is_empty(), "host_times should be populated after a request");
    }
}
