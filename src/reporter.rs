use std::collections::HashMap;
use std::path::Path;

use colored::Colorize;
use serde_json::json;

use crate::models::{CheckResult, ValidationReport};

// ---------------------------------------------------------------------------
// Human-readable
// ---------------------------------------------------------------------------

pub fn report_human(report: &ValidationReport, no_color: bool) {
    if no_color {
        colored::control::set_override(false);
    }

    let broken = report.broken();
    let redirects = report.redirects();
    let total = report.results.len();
    let pages = report.pages_crawled;

    let mode_label = if report.mode == "url" { "URL" } else { "local folder" };
    eprintln!();
    eprintln!(
        "{} — {}: {}",
        "Site Validator".bold(),
        mode_label,
        report.target.cyan()
    );
    eprintln!();

    if !broken.is_empty() {
        eprintln!(
            "{}  {}",
            format!(
                "✗ {} broken link{}",
                broken.len(),
                if broken.len() == 1 { "" } else { "s" }
            )
            .red()
            .bold(),
            format!("out of {total} unique links across {pages} page{}", if pages == 1 { "" } else { "s" }).dimmed()
        );
        eprintln!();
        print_broken_table(&broken, report);
    } else {
        eprintln!(
            "{}  {}",
            format!("✓ All {total} links OK").green().bold(),
            format!(
                "{pages} page{} crawled in {:.1}s",
                if pages == 1 { "" } else { "s" },
                report.duration_secs
            )
            .dimmed()
        );
        eprintln!();
    }

    if !redirects.is_empty() {
        eprintln!(
            "{}",
            format!(
                "↳ {} redirect{}",
                redirects.len(),
                if redirects.len() == 1 { "" } else { "s" }
            )
            .yellow()
        );
        print_redirect_table(&redirects);
    }

    print_summary(report);
}

fn print_broken_table(broken: &[&CheckResult], report: &ValidationReport) {
    let rows: Vec<[String; 5]> = broken
        .iter()
        .flat_map(|r| {
            let url = display_url(&r.url, report);
            r.occurrences.iter().enumerate().map(move |(i, occ)| {
                [
                    if i == 0 { r.status_label() } else { String::new() },
                    if i == 0 { url.clone() } else { String::new() },
                    occ.source.clone(),
                    occ.line.map_or_else(|| "—".to_string(), |l| l.to_string()),
                    occ.text.chars().take(40).collect(),
                ]
            })
        })
        .collect();

    let headers = ["Status", "Broken URL", "Found in", "Line", "Anchor text"];
    print_table(&headers, &rows, &[false, false, false, true, false]);
}

fn print_redirect_table(redirects: &[&CheckResult]) {
    let rows: Vec<[String; 3]> = redirects
        .iter()
        .map(|r| {
            let src = r
                .occurrences
                .first()
                .map(|o| {
                    let extra = if r.occurrences.len() > 1 {
                        format!(" (+{})", r.occurrences.len() - 1)
                    } else {
                        String::new()
                    };
                    format!("{}{}", o.source, extra)
                })
                .unwrap_or_default();
            [r.status_label(), r.url.clone(), src]
        })
        .collect();

    let headers = ["Status", "URL", "Found in"];
    print_table(&headers, &rows, &[false, false, false]);
}

fn print_table<const N: usize>(headers: &[&str; N], rows: &[[String; N]], right_align: &[bool; N]) {
    let widths: Vec<usize> = (0..N)
        .map(|i| {
            rows.iter()
                .map(|r| r[i].len())
                .max()
                .unwrap_or(0)
                .max(headers[i].len())
        })
        .collect();

    // header
    eprint!("  ");
    for (i, h) in headers.iter().enumerate() {
        if right_align[i] {
            eprint!("{:>w$}  ", h.bold(), w = widths[i]);
        } else {
            eprint!("{:<w$}  ", h.bold(), w = widths[i]);
        }
    }
    eprintln!();

    // separator
    eprint!(" ");
    for w in &widths {
        eprint!("{}", "─".repeat(w + 3));
    }
    eprintln!();

    // rows
    for row in rows {
        eprint!("  ");
        for (i, cell) in row.iter().enumerate() {
            if right_align[i] {
                eprint!("{:>w$}  ", cell, w = widths[i]);
            } else {
                eprint!("{:<w$}  ", cell, w = widths[i]);
            }
        }
        eprintln!();
    }
    eprintln!();
}

fn print_summary(report: &ValidationReport) {
    let broken = report.broken().len();
    let redirects = report.redirects().len();
    let ok = report.results.len().saturating_sub(broken + redirects);

    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("{} ok", ok).green().to_string());
    if redirects > 0 {
        parts.push(format!("{} redirect{}", redirects, if redirects == 1 { "" } else { "s" }).yellow().to_string());
    }
    if broken > 0 {
        parts.push(format!("{} broken", broken).red().to_string());
    }
    parts.push(format!("{:.1}s", report.duration_secs).dimmed().to_string());

    eprintln!("  {}", parts.join("  "));
    eprintln!();
}

fn display_url(url: &str, report: &ValidationReport) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    if report.mode == "local" {
        let root = Path::new(&report.target).canonicalize().unwrap_or_else(|_| Path::new(&report.target).to_path_buf());
        if let Ok(rel) = Path::new(url).strip_prefix(&root) {
            return rel.to_string_lossy().to_string();
        }
    }
    url.to_string()
}

// ---------------------------------------------------------------------------
// Site tree (bonus visual)
// ---------------------------------------------------------------------------

pub fn report_tree(report: &ValidationReport, no_color: bool) {
    if no_color {
        colored::control::set_override(false);
    }

    if report.mode == "local" {
        tree_local(report);
    } else {
        tree_url(report);
    }
}

fn tree_local(report: &ValidationReport) {
    let root = Path::new(&report.target)
        .canonicalize()
        .unwrap_or_else(|_| Path::new(&report.target).to_path_buf());

    let root_name = root.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| report.target.clone());

    // Collect pages + broken counts
    let mut pages: HashMap<String, usize> = HashMap::new();
    for result in report.broken() {
        for occ in &result.occurrences {
            *pages.entry(occ.source.clone()).or_insert(0) += 1;
        }
    }
    // Include pages with no broken links too
    for result in &report.results {
        for occ in &result.occurrences {
            pages.entry(occ.source.clone()).or_insert(0);
        }
    }

    let mut paths: Vec<String> = pages.keys().cloned().collect();
    paths.sort();

    eprintln!("\n╭─────────────── Site structure ───────────────╮");
    eprintln!("  {}/", root_name.bold().cyan());

    let last = paths.len().saturating_sub(1);
    for (i, path) in paths.iter().enumerate() {
        let broken_count = pages[path];
        let is_last = i == last;
        let connector = if is_last { "└──" } else { "├──" };

        let label = if broken_count > 0 {
            format!("{} ({})", path.red(), format!("{} broken", broken_count).red().dimmed())
        } else {
            format!("{}", path.green())
        };
        eprintln!("  {} {}", connector.dimmed(), label);
    }
    eprintln!("╰───────────────────────────────────────────────╯");
}

fn tree_url(report: &ValidationReport) {
    use url::Url;

    let base = Url::parse(&report.target).ok();
    let host = base.as_ref().and_then(|u| u.host_str()).unwrap_or(&report.target);

    let mut pages: HashMap<String, usize> = HashMap::new();
    for result in report.broken() {
        for occ in &result.occurrences {
            *pages.entry(occ.source.clone()).or_insert(0) += 1;
        }
    }
    for result in &report.results {
        for occ in &result.occurrences {
            pages.entry(occ.source.clone()).or_insert(0);
        }
    }

    let mut paths: Vec<(String, usize)> = pages.into_iter().collect();
    paths.sort_by(|a, b| a.0.cmp(&b.0));

    eprintln!("\n╭─────────────── Site structure ───────────────╮");
    eprintln!("  {}", host.bold().cyan());

    let last = paths.len().saturating_sub(1);
    for (i, (page, broken_count)) in paths.iter().enumerate() {
        let path = Url::parse(page).map(|u| u.path().to_string()).unwrap_or_else(|_| page.clone());
        let is_last = i == last;
        let connector = if is_last { "└──" } else { "├──" };

        let label = if *broken_count > 0 {
            format!("{} ({})", path.red(), format!("{} broken", broken_count).red().dimmed())
        } else {
            format!("{}", path.green())
        };
        eprintln!("  {} {}", connector.dimmed(), label);
    }
    eprintln!("╰───────────────────────────────────────────────╯");
}

// ---------------------------------------------------------------------------
// JSON
// ---------------------------------------------------------------------------


pub fn report_json(report: &ValidationReport) {
    let broken = report.broken();
    let out = json!({
        "target": report.target,
        "mode": report.mode,
        "summary": {
            "pages_crawled": report.pages_crawled,
            "links_checked": report.results.len(),
            "broken_count": broken.len(),
            "redirect_count": report.redirects().len(),
            "ok_count": report.results.len().saturating_sub(broken.len() + report.redirects().len()),
            "duration_seconds": (report.duration_secs * 1000.0).round() / 1000.0,
            "success": report.is_success(),
        },
        "broken_links": broken.iter().map(|r| {
            json!({
                "url": display_url(&r.url, report),
                "status_code": r.status_code,
                "error": r.error,
                "response_time_ms": (r.response_time_ms * 10.0).round() / 10.0,
                "occurrences": r.occurrences.iter().map(|o| json!({
                    "source": o.source,
                    "line": o.line,
                    "anchor_text": o.text,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
        "redirects": report.redirects().iter().map(|r| {
            json!({
                "url": r.url,
                "status_code": r.status_code,
                "occurrences": r.occurrences.iter().map(|o| json!({
                    "source": o.source,
                    "line": o.line,
                })).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}

// ---------------------------------------------------------------------------
// GitHub Actions annotations
// ---------------------------------------------------------------------------

pub fn report_github(report: &ValidationReport) {
    for result in report.broken() {
        for occ in &result.occurrences {
            let display = display_url(&result.url, report);
            let status = result.status_label();
            let msg = if result.error.is_some() {
                format!("Broken link ({}): {}", status, display)
            } else {
                format!("Broken link ({}): {}", status, display)
            };

            if occ.source.starts_with("http://") || occ.source.starts_with("https://") {
                println!("::error title=Broken link::{}  — found at {} line {}", msg, occ.source, occ.line.unwrap_or(1));
            } else {
                println!("::error file={},line={}::{}", occ.source, occ.line.unwrap_or(1), msg);
            }
        }
    }

    if report.is_success() {
        println!(
            "::notice::All {} links OK ({} pages, {:.1}s)",
            report.results.len(),
            report.pages_crawled,
            report.duration_secs
        );
    }
}

// ---------------------------------------------------------------------------
// TAP
// ---------------------------------------------------------------------------

pub fn report_tap(report: &ValidationReport) {
    println!("1..{}", report.results.len());
    for (i, result) in report.results.iter().enumerate() {
        if result.is_broken() {
            let occ = result.occurrences.first();
            let directive = occ
                .map(|o| format!(" # {}:{}", o.source, o.line.map_or_else(|| "?".to_string(), |l| l.to_string())))
                .unwrap_or_default();
            println!("not ok {} - {} ({}){}", i + 1, result.url, result.status_label(), directive);
        } else {
            println!("ok {} - {} ({})", i + 1, result.url, result.status_label());
        }
    }
}
