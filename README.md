# site-validator

A fast broken-link checker for websites and local HTML directories.

## Features

- Crawl a live site by URL or a local folder of HTML files
- Concurrent link checking with configurable parallelism
- Multiple output formats: human-readable, JSON, GitHub Actions annotations, TAP
- Optional site-structure tree view
- Skip external links, limit crawl depth, disable SSL verification

## Installation

```sh
cargo install --path .
```

The binary is named `sv`.

## Usage

```
sv [OPTIONS] <TARGET>
```

`TARGET` is either a URL (`https://example.com`) or a local directory path.

### Examples

```sh
# Check a live site
sv https://example.com

# Check a local build directory
sv ./dist

# Output as JSON
sv --format json https://example.com

# Skip external links, limit concurrency
sv --no-external -j 10 https://example.com

# Print a tree of the site structure
sv --tree ./site

# Use in CI (GitHub Actions annotations)
sv --format github https://example.com

# Quiet mode — no progress, just results
sv -q --format tap https://example.com
```

### Options

| Flag | Default | Description |
|------|---------|-------------|
| `-f, --format` | `human` | Output format: `human`, `json`, `github`, `tap` |
| `--tree` | off | Print site structure tree (human format only) |
| `--no-external` | off | Skip external links |
| `--max-depth` | `-1` | Max crawl depth for URL mode (`-1` = unlimited) |
| `-j, --concurrency` | `20` | Max concurrent HTTP requests |
| `--timeout` | `10` | Per-request timeout in seconds |
| `--user-agent` | `site-validator/0.1` | HTTP User-Agent string |
| `--no-verify` | off | Disable SSL certificate verification |
| `--no-color` | off | Disable color output |
| `-q, --quiet` | off | Suppress progress output |

## Exit Codes

| Code | Meaning |
|------|---------|
| `0` | All links OK |
| `1` | Broken links found |
| `2` | Fatal error |
