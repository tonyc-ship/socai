"""Task-scoped tabs on top of one long-lived CDP browser connection."""

from __future__ import annotations

import time
import uuid
from dataclasses import dataclass, field
from typing import Any, Callable

from .browser import BrowserSession
from .endpoint import (
    Endpoint,
    discover_existing_chrome_endpoint,
    open_remote_debugging_page,
    resolve_explicit_endpoint,
    wait_for_existing_chrome_endpoint,
)
from .page import PageSession


TaskEventHandler = Callable[[str], None]


@dataclass
class BrowserTaskSession:
    task_id: str
    page: PageSession
    target_id: str
    start_url: str = "about:blank"
    label: str = ""
    site: str = ""
    created_at: float = field(default_factory=time.time)
    status: str = "active"
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "task_id": self.task_id,
            "target_id": self.target_id,
            "start_url": self.start_url,
            "label": self.label,
            "site": self.site,
            "created_at": self.created_at,
            "status": self.status,
            "metadata": self.metadata,
        }


class BrowserTaskSessionManager:
    """Own one browser CDP connection and create one tab per task.

    The Tauri/Python app should keep one manager alive while the app process is
    alive. Individual tasks call create_task(), which opens a new tab over the
    existing CDP socket instead of asking Chrome for a new debugging session.
    """

    def __init__(
        self,
        *,
        browser: BrowserSession | None = None,
        endpoint: Endpoint | None = None,
        browser_ws_url: str | None = None,
        http_url: str | None = None,
        inspect_timeout: float = 45.0,
        open_inspect_when_needed: bool = True,
        on_event: TaskEventHandler | None = None,
    ):
        self.browser = browser
        self.endpoint = endpoint or resolve_explicit_endpoint(browser_ws_url=browser_ws_url, http_url=http_url)
        self.browser_ws_url = browser_ws_url
        self.http_url = http_url
        self.inspect_timeout = inspect_timeout
        self.open_inspect_when_needed = open_inspect_when_needed
        self.on_event = on_event
        self.tasks: dict[str, BrowserTaskSession] = {}

    def _emit(self, message: str) -> None:
        if self.on_event:
            self.on_event(message)

    def _resolve_user_chrome_endpoint(self) -> Endpoint:
        if self.endpoint is not None:
            return self.endpoint

        endpoint = discover_existing_chrome_endpoint()
        if endpoint is None and self.open_inspect_when_needed:
            self._emit("No CDP endpoint found for the logged-in Chrome. Opening remote-debugging setup page.")
            self._emit("In your existing Chrome profile, approve remote debugging if prompted.")
            open_remote_debugging_page()
            endpoint = wait_for_existing_chrome_endpoint(timeout=self.inspect_timeout)

        if endpoint is None:
            raise RuntimeError(
                "Could not find CDP for your existing logged-in Chrome profile. "
                "Open chrome://inspect/#remote-debugging in that Chrome and approve remote debugging, then rerun."
            )

        self.endpoint = endpoint
        self._emit(f"Reusing existing Chrome CDP endpoint from {endpoint.source}")
        return endpoint

    async def ensure_browser(self) -> BrowserSession:
        if self.browser is not None:
            return self.browser

        endpoint = self._resolve_user_chrome_endpoint()
        self.browser = await BrowserSession.connect(endpoint=endpoint)
        return self.browser

    async def create_task(
        self,
        *,
        start_url: str = "about:blank",
        label: str = "",
        site: str = "",
        metadata: dict[str, Any] | None = None,
    ) -> BrowserTaskSession:
        browser = await self.ensure_browser()
        page = await browser.new_page(start_url)
        task = BrowserTaskSession(
            task_id=uuid.uuid4().hex,
            page=page,
            target_id=page.target_id,
            start_url=start_url,
            label=label,
            site=site,
            metadata=dict(metadata or {}),
        )
        self.tasks[task.task_id] = task
        return task

    def get_task(self, task_id: str) -> BrowserTaskSession | None:
        return self.tasks.get(task_id)

    def list_tasks(self) -> list[dict[str, Any]]:
        return [task.to_dict() for task in self.tasks.values()]

    async def close_task(self, task_id: str) -> bool:
        task = self.tasks.get(task_id)
        if task is None:
            return False

        task.status = "closed"
        browser = await self.ensure_browser()
        closed = await browser.close_page(task.target_id)
        self.tasks.pop(task_id, None)
        return closed

    async def shutdown(self) -> None:
        if self.browser is not None:
            await self.browser.stop()
            self.browser = None
        for task in self.tasks.values():
            task.status = "closed"
        self.tasks.clear()
