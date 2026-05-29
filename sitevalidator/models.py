from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime
from typing import Optional


@dataclass
class Occurrence:
    """Where a link was found in source."""
    source: str           # URL or file path of the containing page
    line: Optional[int]   # Line number in source HTML
    text: str             # Anchor text (truncated)


@dataclass
class CheckResult:
    """Result of checking a single URL."""
    url: str
    status_code: Optional[int] = None
    error: Optional[str] = None
    response_time_ms: float = 0.0
    occurrences: list[Occurrence] = field(default_factory=list)

    @property
    def is_broken(self) -> bool:
        if self.error:
            return True
        return self.status_code is not None and self.status_code >= 400

    @property
    def is_redirect(self) -> bool:
        return (
            not self.error
            and self.status_code is not None
            and 300 <= self.status_code < 400
        )

    @property
    def is_ok(self) -> bool:
        return not self.is_broken and not self.is_redirect

    @property
    def status_label(self) -> str:
        if self.error:
            return self.error[:60]
        return str(self.status_code or "?")


@dataclass
class ValidationReport:
    """Aggregate result of a validation run."""
    target: str
    mode: str             # 'url' | 'local'
    pages_crawled: int
    results: list[CheckResult]
    start_time: datetime
    end_time: datetime

    @property
    def duration(self) -> float:
        return (self.end_time - self.start_time).total_seconds()

    @property
    def broken(self) -> list[CheckResult]:
        return [r for r in self.results if r.is_broken]

    @property
    def redirects(self) -> list[CheckResult]:
        return [r for r in self.results if r.is_redirect]

    @property
    def success(self) -> bool:
        return len(self.broken) == 0
