use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};

use crate::checker::check_urls;
use crate::crawler::{crawl_local, crawl_url};
use crate::models::{Occurrence, ValidationReport};
use crate::reporter::{report_github, report_human, report_json, report_tap, report_tree};

#[derive(Debug, Clone, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
    Github,
    Tap,
}

/// Validate TARGET for broken links.
///
/// TARGET can be a URL (https://example.com) or a local folder path.
///
/// Exit codes: 0 = all OK, 1 = broken links found, 2 = fatal error.
#[derive(Parser, Debug)]
#[command(name = "sv", version, about, long_about = None)]
pub struct Args {
    /// URL or local folder to validate
    pub target: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "human")]
    pub format: OutputFormat,

    /// Print a visual tree of the site structure (human format only)
    #[arg(long)]
    pub tree: bool,

    /// Skip external links
    #[arg(long)]
    pub no_external: bool,

    /// Max crawl depth for URL mode (-1 = unlimited)
    #[arg(long, default_value = "-1")]
    pub max_depth: i32,

    /// Max concurrent HTTP requests
    #[arg(short = 'j', long, default_value = "20")]
    pub concurrency: usize,

    /// Per-request timeout in seconds
    #[arg(long, default_value = "10")]
    pub timeout: f64,

    /// HTTP User-Agent
    #[arg(long, default_value = "site-validator/0.1")]
    pub user_agent: String,

    /// Disable SSL certificate verification
    #[arg(long)]
    pub no_verify: bool,

    /// Disable color output
    #[arg(long)]
    pub no_color: bool,

    /// Suppress progress output
    #[arg(short, long)]
    pub quiet: bool,
}

pub async fn run() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.no_color || args.quiet {
        colored::control::set_override(false);
    }

    let is_url = args.target.starts_with("http://") || args.target.starts_with("https://");

    // ---- crawl phase ----
    let t0 = Instant::now();

    let crawl_data = if is_url {
        let pb = if !args.quiet {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} Crawling pages… {msg}")
                    .unwrap(),
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(80));
            Some(pb)
        } else {
            None
        };

        let data = crawl_url(
            &args.target,
            !args.no_external,
            args.max_depth,
            args.concurrency,
            pb.clone(),
        )
        .await?;

        if let Some(pb) = pb {
            pb.finish_and_clear();
        }
        data
    } else {
        let dir = Path::new(&args.target);
        if !dir.exists() {
            anyhow::bail!("path does not exist: {}", args.target);
        }
        if !dir.is_dir() {
            anyhow::bail!("not a directory: {}", args.target);
        }

        let pb = if !args.quiet {
            let pb = ProgressBar::new_spinner();
            pb.set_style(
                ProgressStyle::default_spinner()
                    .template("{spinner:.cyan} Reading HTML files… {msg}")
                    .unwrap(),
            );
            pb.enable_steady_tick(std::time::Duration::from_millis(80));
            Some(pb)
        } else {
            None
        };

        let data = crawl_local(dir, !args.no_external, pb.clone());

        if let Some(pb) = pb {
            pb.finish_and_clear();
        }
        data
    };

    if crawl_data.pages_crawled == 0 {
        eprintln!("Warning: no HTML pages found in {}", args.target);
        return Ok(());
    }

    // ---- deduplicate links ----
    let mut url_occurrences: HashMap<String, Vec<Occurrence>> = HashMap::new();
    for link in crawl_data.links {
        url_occurrences
            .entry(link.url)
            .or_default()
            .push(Occurrence {
                source: link.source,
                line: link.line,
                text: link.text,
            });
    }

    let total_unique = url_occurrences.len();

    if !args.quiet {
        eprintln!(
            "\x1b[2mFound {} unique link{} across {} page{}. Checking…\x1b[0m",
            total_unique,
            if total_unique == 1 { "" } else { "s" },
            crawl_data.pages_crawled,
            if crawl_data.pages_crawled == 1 { "" } else { "s" },
        );
    }

    // ---- check phase ----
    let pb = if !args.quiet {
        let pb = ProgressBar::new(total_unique as u64);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.cyan} {bar:30.cyan/blue} {pos}/{len} {msg}")
                .unwrap()
                .progress_chars("█▉▊▋▌▍▎▏  "),
        );
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        Some(pb)
    } else {
        None
    };

    let results = check_urls(
        url_occurrences,
        args.concurrency,
        args.timeout,
        &args.user_agent,
        !args.no_verify,
        pb.clone(),
    )
    .await;

    if let Some(pb) = pb {
        pb.finish_and_clear();
    }

    let duration_secs = t0.elapsed().as_secs_f64();

    let report = ValidationReport {
        target: args.target.clone(),
        mode: crawl_data.mode,
        pages_crawled: crawl_data.pages_crawled,
        results,
        duration_secs,
    };

    // ---- output ----
    match args.format {
        OutputFormat::Human => {
            report_human(&report, args.no_color);
            if args.tree {
                report_tree(&report, args.no_color);
            }
        }
        OutputFormat::Json => report_json(&report),
        OutputFormat::Github => report_github(&report),
        OutputFormat::Tap => report_tap(&report),
    }

    if !report.is_success() {
        std::process::exit(1);
    }

    Ok(())
}
