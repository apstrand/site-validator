"""CLI entry point for site-validator."""
from __future__ import annotations

import asyncio
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Optional
from urllib.parse import urlparse

import click
from rich.console import Console
from rich.progress import Progress, SpinnerColumn, TextColumn, BarColumn, TaskProgressColumn, TimeElapsedColumn

from .crawler import CrawlData, ExtractedLink, crawl_local, crawl_url
from .checker import check_urls
from .models import CheckResult, Occurrence, ValidationReport
from .reporter import report_github, report_human, report_json, report_tap, report_tree


def _is_url(target: str) -> bool:
    parsed = urlparse(target)
    return parsed.scheme in ("http", "https")


def _build_report(
    target: str,
    crawl_data: CrawlData,
    check_results: list[CheckResult],
    start_time: datetime,
) -> ValidationReport:
    return ValidationReport(
        target=target,
        mode=crawl_data.mode,
        pages_crawled=crawl_data.pages_crawled,
        results=check_results,
        start_time=start_time,
        end_time=datetime.now(timezone.utc),
    )


async def _run(
    target: str,
    *,
    check_external: bool,
    max_depth: int,
    concurrency: int,
    timeout: float,
    user_agent: str,
    verify_ssl: bool,
    quiet: bool,
    no_color: bool,
) -> ValidationReport:
    console = Console(stderr=True, no_color=no_color, quiet=quiet)
    start_time = datetime.now(timezone.utc)

    # ---- crawl phase ----
    crawl_data: Optional[CrawlData] = None

    if _is_url(target):
        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            TimeElapsedColumn(),
            console=console,
            transient=True,
        ) as progress:
            task_id = progress.add_task("Crawling pages…", total=None)
            pages_seen = [0]

            def on_page(url: str) -> None:
                pages_seen[0] += 1
                progress.update(task_id, description=f"Crawling page {pages_seen[0]}: {_trim(url, 60)}")

            crawl_data = await crawl_url(
                target,
                check_external=check_external,
                max_depth=max_depth,
                concurrency=concurrency,
                timeout=timeout,
                user_agent=user_agent,
                on_page=on_page,
                verify_ssl=verify_ssl,
            )
    else:
        directory = Path(target).resolve()
        if not directory.exists():
            console.print(f"[red]Error:[/red] path does not exist: {directory}", highlight=False)
            sys.exit(2)
        if not directory.is_dir():
            console.print(f"[red]Error:[/red] not a directory: {directory}", highlight=False)
            sys.exit(2)

        with Progress(
            SpinnerColumn(),
            TextColumn("[progress.description]{task.description}"),
            console=console,
            transient=True,
        ) as progress:
            task_id = progress.add_task("Reading HTML files…", total=None)

            def on_page(rel: str) -> None:
                progress.update(task_id, description=f"Reading: {_trim(rel, 60)}")

            crawl_data = crawl_local(
                directory,
                check_external=check_external,
                on_page=on_page,
            )

    if crawl_data.pages_crawled == 0:
        console.print("[yellow]Warning:[/yellow] no HTML pages found", highlight=False)
        return ValidationReport(
            target=target,
            mode=crawl_data.mode,
            pages_crawled=0,
            results=[],
            start_time=start_time,
            end_time=datetime.now(timezone.utc),
        )

    # ---- deduplicate links ----
    url_occurrences: dict[str, list[Occurrence]] = defaultdict(list)
    for link in crawl_data.links:
        url_occurrences[link.url].append(link.occurrence)

    total_unique = len(url_occurrences)
    console.print(
        f"[dim]Found {total_unique} unique link{'s' if total_unique!=1 else ''} across "
        f"{crawl_data.pages_crawled} page{'s' if crawl_data.pages_crawled!=1 else ''}. Checking…[/dim]",
        highlight=False,
    )

    # ---- check phase ----
    checked_so_far = [0]

    with Progress(
        SpinnerColumn(),
        TextColumn("[progress.description]{task.description}"),
        BarColumn(),
        TaskProgressColumn(),
        TimeElapsedColumn(),
        console=console,
        transient=True,
    ) as progress:
        task_id = progress.add_task("Checking links…", total=total_unique)

        def on_checked(result: CheckResult) -> None:
            checked_so_far[0] += 1
            icon = "[red]✗[/red]" if result.is_broken else "[green]✓[/green]"
            progress.update(
                task_id,
                advance=1,
                description=f"Checking links… {icon} {_trim(result.url, 50)}",
            )

        check_results = await check_urls(
            dict(url_occurrences),
            concurrency=concurrency,
            timeout=timeout,
            user_agent=user_agent,
            verify_ssl=verify_ssl,
            on_checked=on_checked,
        )

    return _build_report(target, crawl_data, check_results, start_time)


def _trim(s: str, n: int) -> str:
    return s if len(s) <= n else "…" + s[-(n - 1):]


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------

@click.command(context_settings={"help_option_names": ["-h", "--help"]})
@click.argument("target")
@click.option(
    "--format", "-f",
    "output_format",
    type=click.Choice(["human", "json", "github", "tap"], case_sensitive=False),
    default="human",
    show_default=True,
    help="Output format. 'github' emits GitHub Actions annotations.",
)
@click.option(
    "--output", "-o",
    type=click.Path(dir_okay=False, writable=True),
    default=None,
    help="Write report to FILE instead of stdout.",
)
@click.option(
    "--tree", "show_tree",
    is_flag=True,
    default=False,
    help="Print a visual tree of the site structure (human format only).",
)
@click.option(
    "--no-external",
    is_flag=True,
    default=False,
    help="Skip checking external (off-domain / off-tree) links.",
)
@click.option(
    "--max-depth",
    type=int,
    default=-1,
    metavar="N",
    help="Max crawl depth for URL mode (-1 = unlimited).",
)
@click.option(
    "--concurrency", "-j",
    type=int,
    default=10,
    show_default=True,
    help="Max concurrent HTTP requests.",
)
@click.option(
    "--timeout",
    type=float,
    default=10.0,
    show_default=True,
    metavar="SECS",
    help="Per-request timeout in seconds.",
)
@click.option(
    "--user-agent",
    default="site-validator/0.1",
    show_default=True,
    help="HTTP User-Agent header.",
)
@click.option(
    "--no-verify",
    is_flag=True,
    default=False,
    help="Disable SSL certificate verification.",
)
@click.option(
    "--no-color",
    is_flag=True,
    default=False,
    help="Disable color output.",
)
@click.option(
    "--quiet", "-q",
    is_flag=True,
    default=False,
    help="Suppress progress output (implies --no-color for stderr).",
)
@click.version_option(package_name="site-validator")
def main(
    target: str,
    output_format: str,
    output: Optional[str],
    show_tree: bool,
    no_external: bool,
    max_depth: int,
    concurrency: int,
    timeout: float,
    user_agent: str,
    no_verify: bool,
    no_color: bool,
    quiet: bool,
) -> None:
    """Validate TARGET for broken links.

    TARGET can be a URL (https://example.com) or a local folder path.

    \b
    Exit codes:
      0  All links OK
      1  One or more broken links found
      2  Fatal error (invalid target, network failure, etc.)

    \b
    Examples:
      sv https://example.com
      sv ./dist/ --format github
      sv https://example.com --format json -o report.json
      sv ./site --tree --no-external
    """
    report = asyncio.run(
        _run(
            target,
            check_external=not no_external,
            max_depth=max_depth,
            concurrency=concurrency,
            timeout=timeout,
            user_agent=user_agent,
            verify_ssl=not no_verify,
            quiet=quiet,
            no_color=no_color,
        )
    )

    # ---- output ----
    out_file = open(output, "w", encoding="utf-8") if output else None
    console = Console(file=out_file, no_color=no_color or bool(output))

    try:
        fmt = output_format.lower()
        if fmt == "human":
            report_human(report, console)
            if show_tree:
                report_tree(report, console)
        elif fmt == "json":
            report_json(report, file=out_file or sys.stdout)
        elif fmt == "github":
            report_github(report, file=out_file or sys.stdout)
        elif fmt == "tap":
            report_tap(report, file=out_file or sys.stdout)
    finally:
        if out_file:
            out_file.close()

    sys.exit(0 if report.success else 1)
