use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use indicatif::ProgressBar;
use reqwest::Client;
use url::Url;
use walkdir::WalkDir;

use crate::extractor::{extract_links_local, extract_links_url, ExtractedLink};

pub struct CrawlData {
    pub mode: &'static str,
    pub pages_crawled: usize,
    pub links: Vec<ExtractedLink>,
}

// ---------------------------------------------------------------------------
// URL mode
// ---------------------------------------------------------------------------

pub async fn crawl_url(
    start: &str,
    check_external: bool,
    max_depth: i32,
    concurrency: usize,
    pb: Option<ProgressBar>,
) -> anyhow::Result<CrawlData> {
    let start_url = Url::parse(start)?;
    let origin = start_url.host_str().unwrap_or("").to_string();

    let client = Client::builder()
        .user_agent("site-validator/0.1")
        .timeout(std::time::Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::limited(10))
        .danger_accept_invalid_certs(true)
        .build()?;

    let visited: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));
    let all_links: Arc<Mutex<Vec<ExtractedLink>>> = Arc::new(Mutex::new(Vec::new()));
    let pages_crawled = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut current_level = vec![normalize_url(start)];
    let mut depth = 0usize;

    while !current_level.is_empty() {
        if max_depth >= 0 && depth > max_depth as usize {
            break;
        }

        // Deduplicate and filter within-origin
        let to_fetch: Vec<String> = {
            let mut v = visited.lock().unwrap();
            current_level
                .into_iter()
                .filter(|u| {
                    !v.contains(u) && {
                        v.insert(u.clone());
                        true
                    }
                })
                .filter(|u| is_same_origin(u, &origin))
                .collect()
        };

        if to_fetch.is_empty() {
            break;
        }

        // Fetch all pages at this depth level concurrently
        let next_level: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

        futures::stream::iter(to_fetch)
            .map(|url| {
                let client = client.clone();
                let origin = origin.clone();
                let visited = visited.clone();
                let all_links = all_links.clone();
                let next_level = next_level.clone();
                let pages_crawled = pages_crawled.clone();
                let pb = pb.clone();
                let check_external = check_external;

                async move {
                    let source = url.clone();
                    let Ok(resp) = client.get(&url).send().await else { return };
                    let ct = resp
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();

                    if !ct.contains("html") {
                        return;
                    }

                    let Ok(body) = resp.bytes().await else { return };

                    let count = pages_crawled.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    if let Some(ref pb) = pb {
                        pb.set_message(format!("page {}: {}", count, truncate(&source, 55)));
                    }

                    let links = extract_links_url(&body, &source, &source);

                    // Queue new internal pages
                    {
                        let v = visited.lock().unwrap();
                        let mut nxt = next_level.lock().unwrap();
                        for link in &links {
                            if is_same_origin(&link.url, &origin) && !v.contains(&link.url) {
                                nxt.push(link.url.clone());
                            }
                        }
                    }

                    // Collect all links (internal + external if requested)
                    {
                        let mut al = all_links.lock().unwrap();
                        for link in links {
                            if check_external || is_same_origin(&link.url, &origin) {
                                al.push(link);
                            }
                        }
                    }
                }
            })
            .buffer_unordered(concurrency)
            .collect::<Vec<_>>()
            .await;

        current_level = Arc::try_unwrap(next_level)
            .unwrap()
            .into_inner()
            .unwrap();
        depth += 1;
    }

    let pages = pages_crawled.load(std::sync::atomic::Ordering::Relaxed);
    let links = Arc::try_unwrap(all_links).unwrap().into_inner().unwrap();

    Ok(CrawlData {
        mode: "url",
        pages_crawled: pages,
        links,
    })
}

fn is_same_origin(url: &str, origin: &str) -> bool {
    Url::parse(url)
        .map(|u| u.host_str().unwrap_or("") == origin)
        .unwrap_or(false)
}

fn normalize_url(url: &str) -> String {
    Url::parse(url)
        .map(|mut u| {
            u.set_fragment(None);
            u.to_string()
        })
        .unwrap_or_else(|_| url.to_string())
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("…{}", &s[s.len().saturating_sub(n - 1)..])
    }
}

// ---------------------------------------------------------------------------
// Local folder mode
// ---------------------------------------------------------------------------

pub fn crawl_local(
    directory: &Path,
    check_external: bool,
    pb: Option<ProgressBar>,
) -> CrawlData {
    let root = match directory.canonicalize() {
        Ok(p) => p,
        Err(_) => directory.to_path_buf(),
    };

    let html_files: Vec<PathBuf> = WalkDir::new(&root)
        .follow_links(true)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_type().is_file()
                && e.path()
                    .extension()
                    .map(|x| x == "html" || x == "htm")
                    .unwrap_or(false)
        })
        .map(|e| e.into_path())
        .collect();

    let mut all_links: Vec<ExtractedLink> = Vec::new();

    for file in &html_files {
        let rel = file.strip_prefix(&root).unwrap_or(file);
        let source = rel.to_string_lossy().to_string();

        if let Some(ref pb) = pb {
            pb.set_message(source.clone());
        }

        let Ok(bytes) = std::fs::read(file) else { continue };
        let links = extract_links_local(&bytes, file, &root, &source, check_external);
        all_links.extend(links);
    }

    CrawlData {
        mode: "local",
        pages_crawled: html_files.len(),
        links: all_links,
    }
}
