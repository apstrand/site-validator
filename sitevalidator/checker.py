"""Check whether URLs are reachable and return their status."""
from __future__ import annotations

import asyncio
import time
from typing import Callable, Optional

import httpx

from .models import CheckResult, Occurrence


async def check_urls(
    url_occurrences: dict[str, list[Occurrence]],
    *,
    concurrency: int = 10,
    timeout: float = 10.0,
    user_agent: str = "site-validator/0.1",
    verify_ssl: bool = True,
    on_checked: Optional[Callable[[CheckResult], None]] = None,
) -> list[CheckResult]:
    sem = asyncio.Semaphore(concurrency)
    headers = {"User-Agent": user_agent}

    async def check_one(url: str, occurrences: list[Occurrence]) -> CheckResult:
        async with sem:
            # Skip non-http paths (local file paths)
            if not url.startswith(("http://", "https://")):
                return _check_local_path(url, occurrences)

            t0 = time.monotonic()
            try:
                async with httpx.AsyncClient(
                    headers=headers,
                    timeout=httpx.Timeout(timeout),
                    follow_redirects=False,
                    verify=verify_ssl,
                ) as client:
                    # Try HEAD first (cheap), fall back to GET if server rejects it
                    resp = await client.head(url)
                    if resp.status_code == 405:
                        resp = await client.get(url)

                elapsed = (time.monotonic() - t0) * 1000
                result = CheckResult(
                    url=url,
                    status_code=resp.status_code,
                    response_time_ms=elapsed,
                    occurrences=occurrences,
                )
            except httpx.TimeoutException:
                result = CheckResult(
                    url=url,
                    error="timeout",
                    response_time_ms=(time.monotonic() - t0) * 1000,
                    occurrences=occurrences,
                )
            except httpx.ConnectError as e:
                result = CheckResult(
                    url=url,
                    error=f"connect error: {_short_error(e)}",
                    occurrences=occurrences,
                )
            except httpx.TooManyRedirects:
                result = CheckResult(
                    url=url,
                    error="too many redirects",
                    occurrences=occurrences,
                )
            except Exception as e:
                result = CheckResult(
                    url=url,
                    error=_short_error(e),
                    occurrences=occurrences,
                )

            if on_checked:
                on_checked(result)
            return result

    tasks = [
        check_one(url, occurrences)
        for url, occurrences in url_occurrences.items()
    ]
    return list(await asyncio.gather(*tasks))


def _check_local_path(url: str, occurrences: list[Occurrence]) -> CheckResult:
    """Check that a local file path exists."""
    from pathlib import Path

    path = Path(url)
    if path.exists():
        status = 200
        err = None
    else:
        # Try common index file extensions for directories
        if path.suffix == "" and (path / "index.html").exists():
            status = 200
            err = None
        else:
            status = 404
            err = None

    return CheckResult(
        url=url,
        status_code=status,
        error=err,
        occurrences=occurrences,
    )


def _short_error(e: Exception) -> str:
    msg = str(e)
    return msg[:80] if msg else type(e).__name__
