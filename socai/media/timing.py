"""Timing tracker for media-pipeline diagnostics.

Used by ``MediaProcessor`` and its sub-processors to accumulate per-operation
counts and durations. Site runtimes and tools (e.g. ``xhs_topic_scan``) read
``summary()`` to surface a phase breakdown alongside their artifacts so slow
runs can be diagnosed without re-running with a profiler.
"""

from __future__ import annotations

import time
from contextlib import contextmanager
from dataclasses import dataclass, field
from threading import Lock
from typing import Iterator


@dataclass
class TimingRecord:
    counts: dict[str, int] = field(default_factory=dict)
    totals: dict[str, float] = field(default_factory=dict)
    _lock: Lock = field(default_factory=Lock, repr=False, compare=False)

    def record(self, op: str, duration: float) -> None:
        if not op:
            return
        with self._lock:
            self.counts[op] = self.counts.get(op, 0) + 1
            self.totals[op] = self.totals.get(op, 0.0) + float(duration)

    @contextmanager
    def measure(self, op: str) -> Iterator[None]:
        t0 = time.perf_counter()
        try:
            yield
        finally:
            self.record(op, time.perf_counter() - t0)

    async def measure_async(self, op: str, coro):
        t0 = time.perf_counter()
        try:
            return await coro
        finally:
            self.record(op, time.perf_counter() - t0)

    def summary(self) -> dict[str, dict[str, float]]:
        with self._lock:
            return {
                op: {
                    "count": self.counts.get(op, 0),
                    "total_s": round(self.totals[op], 3),
                    "avg_s": round(self.totals[op] / self.counts[op], 3)
                    if self.counts.get(op)
                    else 0.0,
                }
                for op in sorted(self.totals)
            }

    def reset(self) -> None:
        with self._lock:
            self.counts.clear()
            self.totals.clear()
