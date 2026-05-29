"""Format and output validation results."""
from __future__ import annotations

import json
import sys
from collections import defaultdict
from pathlib import Path
from typing import Optional

from rich.console import Console
from rich.table import Table
from rich.tree import Tree
from rich import box
from rich.text import Text
from rich.panel import Panel
from rich.columns import Columns

from .models import CheckResult, ValidationReport


def _status_style(result: CheckResult) -> str:
    if result.is_broken:
        return "bold red"
    if result.is_redirect:
        return "yellow"
    return "green"


def _status_text(result: CheckResult) -> Text:
    label = result.status_label
    style = _status_style(result)
    return Text(label, style=style)


# ---------------------------------------------------------------------------
# Human-readable (Rich)
# ---------------------------------------------------------------------------

def report_human(report: ValidationReport, console: Console) -> None:
    broken = report.broken
    redirects = report.redirects
    total = len(report.results)

    # Header
    mode_label = "URL" if report.mode == "url" else "local folder"
    console.print(f"\n[bold]Site Validator[/bold] — {mode_label}: [cyan]{report.target}[/cyan]\n")

    if broken:
        console.print(f"[bold red]✗ {len(broken)} broken link{'s' if len(broken) != 1 else ''}[/bold red]  "
                      f"[dim]out of {total} unique links across {report.pages_crawled} page{'s' if report.pages_crawled != 1 else ''}[/dim]\n")
        _print_broken_table(broken, console, report)
    else:
        console.print(f"[bold green]✓ All {total} links OK[/bold green]  "
                      f"[dim]{report.pages_crawled} page{'s' if report.pages_crawled != 1 else ''} crawled in {report.duration:.1f}s[/dim]\n")

    if redirects:
        console.print(f"[yellow]↳ {len(redirects)} redirect{'s' if len(redirects) != 1 else ''} (warnings)[/yellow]")
        _print_redirect_table(redirects, console)

    # Summary line
    _print_summary(report, console)


def _print_broken_table(broken: list[CheckResult], console: Console, report: ValidationReport) -> None:
    table = Table(box=box.SIMPLE_HEAD, show_header=True, header_style="bold")
    table.add_column("Status", style="bold red", no_wrap=True, min_width=8)
    table.add_column("Broken URL")
    table.add_column("Found in", style="dim")
    table.add_column("Line", style="dim", justify="right", no_wrap=True)
    table.add_column("Anchor text", style="dim italic", max_width=30)

    for result in broken:
        status = result.status_label
        display_url = _display_url(result.url, report)
        for i, occ in enumerate(result.occurrences):
            line_str = str(occ.line) if occ.line else "—"
            source_display = _shorten_source(occ.source)
            table.add_row(
                status if i == 0 else "",
                display_url if i == 0 else "",
                source_display,
                line_str,
                occ.text[:40] if occ.text else "—",
            )
    console.print(table)


def _print_redirect_table(redirects: list[CheckResult], console: Console) -> None:
    table = Table(box=box.SIMPLE_HEAD, show_header=True, header_style="bold dim")
    table.add_column("Status", style="yellow", no_wrap=True, min_width=8)
    table.add_column("URL")
    table.add_column("Found in", style="dim")

    for result in redirects:
        first_occ = result.occurrences[0] if result.occurrences else None
        source = _shorten_source(first_occ.source) if first_occ else "—"
        extra = f" (+{len(result.occurrences)-1} more)" if len(result.occurrences) > 1 else ""
        table.add_row(str(result.status_code), result.url, source + extra)

    console.print(table)


def _print_summary(report: ValidationReport, console: Console) -> None:
    broken_count = len(report.broken)
    ok_count = len(report.results) - broken_count - len(report.redirects)
    parts = []
    parts.append(f"[green]{ok_count} ok[/green]")
    if report.redirects:
        parts.append(f"[yellow]{len(report.redirects)} redirect{'s' if len(report.redirects)!=1 else ''}[/yellow]")
    if broken_count:
        parts.append(f"[red]{broken_count} broken[/red]")
    parts.append(f"[dim]{report.duration:.1f}s[/dim]")
    console.print("  ".join(parts) + "\n")


def _shorten_source(source: str) -> str:
    if len(source) <= 60:
        return source
    return "…" + source[-57:]


def _display_url(url: str, report: ValidationReport) -> str:
    """Return a display-friendly URL, making local paths relative to target."""
    if url.startswith(("http://", "https://")):
        return url
    if report.mode == "local":
        try:
            root = Path(report.target).resolve()
            rel = Path(url).relative_to(root)
            return str(rel)
        except ValueError:
            pass
    return url


# ---------------------------------------------------------------------------
# Site tree (bonus visual report)
# ---------------------------------------------------------------------------

def report_tree(report: ValidationReport, console: Console) -> None:
    """Print a tree view of the site structure."""
    if report.mode == "url":
        _tree_url(report, console)
    else:
        _tree_local(report, console)


def _tree_url(report: ValidationReport, console: Console) -> None:
    broken_urls = {r.url for r in report.broken}

    # Group pages by path segments
    from urllib.parse import urlparse
    pages_by_path: dict[str, list[str]] = defaultdict(list)
    all_pages = set()
    for result in report.results:
        for occ in result.occurrences:
            all_pages.add(occ.source)

    parsed_base = urlparse(report.target)
    tree = Tree(f"[bold cyan]{parsed_base.netloc}[/bold cyan]")

    # Simple flat list sorted by path
    for page in sorted(all_pages):
        pp = urlparse(page)
        path = pp.path or "/"
        broken_on_page = sum(
            1 for r in report.broken
            for occ in r.occurrences if occ.source == page
        )
        if broken_on_page:
            label = f"[red]{path}[/red] [dim red]({broken_on_page} broken)[/dim red]"
        else:
            label = f"[green]{path}[/green]"
        tree.add(label)

    console.print(Panel(tree, title="Site structure", border_style="dim"))


def _tree_local(report: ValidationReport, console: Console) -> None:
    from pathlib import Path as _Path

    all_pages = set()
    for result in report.results:
        for occ in result.occurrences:
            if not occ.source.startswith(("http://", "https://")):
                all_pages.add(occ.source)

    if not all_pages:
        return

    root = _Path(report.target).resolve()
    tree = Tree(f"[bold cyan]{root.name}/[/bold cyan]")

    nodes: dict[str, Tree] = {"": tree}

    for page in sorted(all_pages):
        try:
            rel = str(_Path(page).relative_to(root))
        except ValueError:
            rel = page

        parts = _Path(rel).parts
        parent_key = ""
        for i, part in enumerate(parts):
            key = "/".join(parts[: i + 1])
            if key not in nodes:
                is_last = i == len(parts) - 1
                broken_on_page = sum(
                    1 for r in report.broken
                    for occ in r.occurrences if occ.source == page
                ) if is_last else 0

                if is_last and broken_on_page:
                    label = f"[red]{part}[/red] [dim red]({broken_on_page} broken)[/dim red]"
                elif is_last:
                    label = f"[green]{part}[/green]"
                else:
                    label = f"[bold]{part}/[/bold]"

                nodes[key] = nodes[parent_key].add(label)
            parent_key = key

    console.print(Panel(tree, title="Site structure", border_style="dim"))


# ---------------------------------------------------------------------------
# JSON output
# ---------------------------------------------------------------------------

def report_json(report: ValidationReport, file=None) -> None:
    out = file or sys.stdout
    data = _report_to_dict(report)
    json.dump(data, out, indent=2)
    print(file=out)  # trailing newline


def _report_to_dict(report: ValidationReport) -> dict:
    broken = report.broken
    return {
        "target": report.target,
        "mode": report.mode,
        "summary": {
            "pages_crawled": report.pages_crawled,
            "links_checked": len(report.results),
            "broken_count": len(broken),
            "redirect_count": len(report.redirects),
            "ok_count": len(report.results) - len(broken) - len(report.redirects),
            "duration_seconds": round(report.duration, 3),
            "success": report.success,
        },
        "broken_links": [
            {
                "url": r.url,
                "status_code": r.status_code,
                "error": r.error,
                "response_time_ms": round(r.response_time_ms, 1),
                "occurrences": [
                    {
                        "source": occ.source,
                        "line": occ.line,
                        "anchor_text": occ.text,
                    }
                    for occ in r.occurrences
                ],
            }
            for r in broken
        ],
        "redirects": [
            {
                "url": r.url,
                "status_code": r.status_code,
                "occurrences": [
                    {"source": occ.source, "line": occ.line}
                    for occ in r.occurrences
                ],
            }
            for r in report.redirects
        ],
    }


# ---------------------------------------------------------------------------
# GitHub Actions annotation format
# ---------------------------------------------------------------------------

def report_github(report: ValidationReport, file=None) -> None:
    out = file or sys.stdout
    for result in report.broken:
        for occ in result.occurrences:
            source = occ.source
            line = occ.line or 1
            display = _display_url(result.url, report)
            status = result.status_label
            msg = f"Broken link ({status}): {display}"
            if result.error:
                msg = f"Broken link ({result.error}): {display}"

            if source.startswith(("http://", "https://")):
                print(f"::error title=Broken link::{msg} — found at {source} line {line}", file=out)
            else:
                print(f"::error file={source},line={line}::{msg}", file=out)

    if report.success:
        total = len(report.results)
        print(f"::notice::All {total} links OK ({report.pages_crawled} pages, {report.duration:.1f}s)", file=out)


# ---------------------------------------------------------------------------
# TAP (Test Anything Protocol) — useful for some CI systems
# ---------------------------------------------------------------------------

def report_tap(report: ValidationReport, file=None) -> None:
    out = file or sys.stdout
    results = report.results
    print(f"1..{len(results)}", file=out)
    for i, result in enumerate(results, 1):
        if result.is_broken:
            occ = result.occurrences[0] if result.occurrences else None
            directive = ""
            if occ:
                directive = f" # {occ.source}:{occ.line or '?'}"
            print(f"not ok {i} - {result.url} ({result.status_label}){directive}", file=out)
        else:
            print(f"ok {i} - {result.url} ({result.status_label})", file=out)
