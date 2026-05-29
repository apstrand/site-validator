use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use futures::StreamExt;
use indicatif::ProgressBar;
use reqwest::{Client, StatusCode};

use crate::models::{CheckResult, Occurrence};

pub async fn check_urls(
    url_occurrences: HashMap<String, Vec<Occurrence>>,
    concurrency: usize,
    timeout_secs: f64,
    user_agent: &str,
    verify_ssl: bool,
    pb: Option<ProgressBar>,
) -> Vec<CheckResult> {
    let client = Client::builder()
        .user_agent(user_agent)
        .timeout(std::time::Duration::from_secs_f64(timeout_secs))
        .redirect(reqwest::redirect::Policy::none())
        .danger_accept_invalid_certs(!verify_ssl)
        .build()
        .expect("failed to build HTTP client");

    futures::stream::iter(url_occurrences)
        .map(|(url, occurrences)| {
            let client = client.clone();
            let pb = pb.clone();
            async move {
                let result = check_one(&client, url, occurrences).await;
                if let Some(ref pb) = pb {
                    let icon = if result.is_broken() { "✗" } else { "✓" };
                    pb.inc(1);
                    pb.set_message(format!("{} {}", icon, truncate(&result.url, 55)));
                }
                result
            }
        })
        .buffer_unordered(concurrency)
        .collect()
        .await
}

async fn check_one(client: &Client, url: String, occurrences: Vec<Occurrence>) -> CheckResult {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return check_local_path(url, occurrences);
    }

    let t0 = Instant::now();

    // Try HEAD first, fall back to GET if server rejects it
    let resp = match client.head(&url).send().await {
        Ok(r) if r.status() == StatusCode::METHOD_NOT_ALLOWED => {
            client.get(&url).send().await
        }
        other => other,
    };

    let elapsed = t0.elapsed().as_secs_f64() * 1000.0;

    match resp {
        Ok(r) => CheckResult {
            url,
            status_code: Some(r.status().as_u16()),
            error: None,
            response_time_ms: elapsed,
            occurrences,
        },
        Err(e) => {
            let error = if e.is_timeout() {
                "timeout".to_string()
            } else if e.is_connect() {
                format!("connect error: {}", short_error(&e))
            } else if e.is_redirect() {
                "too many redirects".to_string()
            } else {
                short_error(&e)
            };
            CheckResult {
                url,
                status_code: None,
                error: Some(error),
                response_time_ms: elapsed,
                occurrences,
            }
        }
    }
}

fn check_local_path(url: String, occurrences: Vec<Occurrence>) -> CheckResult {
    let path = Path::new(&url);
    let exists = path.exists()
        || path.with_extension("html").exists()
        || path.join("index.html").exists();

    CheckResult {
        url,
        status_code: Some(if exists { 200 } else { 404 }),
        error: None,
        response_time_ms: 0.0,
        occurrences,
    }
}

fn short_error(e: &reqwest::Error) -> String {
    let s = e.to_string();
    s.chars().take(80).collect()
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("…{}", &s[s.len().saturating_sub(n - 1)..])
    }
}
