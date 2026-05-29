"""Crawl pages and extract links with source locations."""
from __future__ import annotations

import asyncio
from collections import defaultdict
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Optional
from urllib.parse import urljoin, urlparse, urldefrag

import httpx
from lxml import html as lhtml
from lxml.etree import ParserError

from .models import Occurrence


@dataclass
class ExtractedLink:
    url: str
    occurrence: Occurrence


@dataclass
class CrawlData:
    mode: str                         # 'url' | 'local'
    pages_crawled: int
    links: list[ExtractedLink] = field(default_factory=list)


def _normalize_url(url: str) -> str:
    url, _ = urldefrag(url)
    parsed = urlparse(url)
    path = parsed.path or "/"
    return parsed._replace(path=path).geturl()


def _is_same_origin(url: str, base_netloc: str) -> bool:
    return urlparse(url).netloc == base_netloc


def _is_checkable_scheme(url: str) -> bool:
    return urlparse(url).scheme in ("http", "https")


def _extract_links_from_html(
    content: str | bytes, base_url: str, source_label: str
) -> list[ExtractedLink]:
    if isinstance(content, str):
        content = content.encode("utf-8", errors="replace")
    try:
        doc = lhtml.fromstring(content)
    except (ParserError, ValueError):
        return []

    links: list[ExtractedLink] = []
    for tag in doc.iter("a"):
        href = (tag.get("href") or "").strip()
        if not href:
            continue
        scheme = urlparse(href).scheme
        if scheme and scheme not in ("http", "https", ""):
            continue  # skip mailto:, tel:, javascript:, etc.
        if href.startswith("#"):
            continue

        resolved = _normalize_url(urljoin(base_url, href))
        if not _is_checkable_scheme(resolved):
            continue

        line: Optional[int] = tag.sourceline
        # lxml stores text in .text and tail; get_text_content for full text
        text = (tag.text_content() or "").strip()[:120]
        links.append(ExtractedLink(
            url=resolved,
            occurrence=Occurrence(source=source_label, line=line, text=text),
        ))
    return links


# ---------------------------------------------------------------------------
# URL mode
# ---------------------------------------------------------------------------

async def crawl_url(
    start_url: str,
    *,
    check_external: bool = True,
    max_depth: int = -1,
    concurrency: int = 10,
    timeout: float = 10.0,
    user_agent: str = "site-validator/0.1 (https://github.com/)",
    on_page: Optional[Callable[[str], None]] = None,
    verify_ssl: bool = True,
) -> CrawlData:
    start_url = _normalize_url(start_url)
    origin_netloc = urlparse(start_url).netloc

    visited_pages: set[str] = set()
    queued_external: set[str] = set()
    all_links: list[ExtractedLink] = []

    # BFS queue: (url, depth)
    queue: list[tuple[str, int]] = [(start_url, 0)]
    sem = asyncio.Semaphore(concurrency)
    headers = {"User-Agent": user_agent}

    async def fetch(url: str) -> tuple[int, str, bytes]:
        """Return (status_code, content_type, body)."""
        async with sem:
            async with httpx.AsyncClient(
                headers=headers,
                timeout=httpx.Timeout(timeout),
                follow_redirects=True,
                verify=verify_ssl,
            ) as client:
                resp = await client.get(url)
                ct = resp.headers.get("content-type", "")
                return resp.status_code, ct, resp.content if "html" in ct else b""

    pages_crawled = 0

    while queue:
        batch, queue = queue[:], []
        tasks = []
        for url, depth in batch:
            if url in visited_pages:
                continue
            visited_pages.add(url)
            if not _is_same_origin(url, origin_netloc):
                continue  # don't crawl external pages
            if max_depth >= 0 and depth > max_depth:
                continue
            tasks.append((url, depth, asyncio.create_task(fetch(url))))

        for url, depth, task in tasks:
            try:
                status, ct, body = await task
            except Exception:
                continue

            pages_crawled += 1
            if on_page:
                on_page(url)

            if "html" not in ct or not body:
                continue

            links = _extract_links_from_html(body, url, url)
            all_links.extend(links)

            for link in links:
                if _is_same_origin(link.url, origin_netloc):
                    if link.url not in visited_pages:
                        queue.append((link.url, depth + 1))
                elif check_external and link.url not in queued_external:
                    queued_external.add(link.url)

    # Add external links as bare entries (no occurrence duplicates)
    ext_seen: set[str] = set()
    for link in all_links:
        if not _is_same_origin(link.url, origin_netloc):
            ext_seen.add(link.url)

    return CrawlData(mode="url", pages_crawled=pages_crawled, links=all_links)


# ---------------------------------------------------------------------------
# Local folder mode
# ---------------------------------------------------------------------------

def _resolve_local_href(href: str, file_path: Path, root: Path) -> Optional[str]:
    """Resolve a href to a filesystem path string, or return http(s) URL."""
    scheme = urlparse(href).scheme
    if scheme in ("http", "https"):
        return href
    if scheme and scheme not in ("", "file"):
        return None  # mailto, etc.

    if href.startswith("//"):
        return "https:" + href

    # Strip query/fragment
    href_clean, _ = urldefrag(href)
    if "?" in href_clean:
        href_clean = href_clean.split("?", 1)[0]

    if href_clean.startswith("/"):
        resolved = root / href_clean.lstrip("/")
    else:
        resolved = file_path.parent / href_clean

    try:
        return str(resolved.resolve())
    except Exception:
        return None


def crawl_local(
    directory: Path,
    *,
    check_external: bool = True,
    on_page: Optional[Callable[[str], None]] = None,
) -> CrawlData:
    root = directory.resolve()
    html_files = sorted(
        list(root.rglob("*.html")) + list(root.rglob("*.htm"))
    )

    all_links: list[ExtractedLink] = []

    for html_file in html_files:
        if on_page:
            on_page(str(html_file.relative_to(root)))

        try:
            raw = html_file.read_bytes()
        except Exception:
            continue

        try:
            doc = lhtml.fromstring(raw)
        except (ParserError, ValueError):
            continue

        source_label = str(html_file.relative_to(root))

        for tag in doc.iter("a"):
            href = (tag.get("href") or "").strip()
            if not href or href.startswith("#"):
                continue
            scheme = urlparse(href).scheme
            if scheme and scheme not in ("http", "https", "", "file"):
                continue

            resolved = _resolve_local_href(href, html_file, root)
            if resolved is None:
                continue

            if resolved.startswith(("http://", "https://")) and not check_external:
                continue

            line: Optional[int] = tag.sourceline
            text = (tag.text_content() or "").strip()[:120]
            all_links.append(ExtractedLink(
                url=resolved,
                occurrence=Occurrence(source=source_label, line=line, text=text),
            ))

    return CrawlData(mode="local", pages_crawled=len(html_files), links=all_links)
