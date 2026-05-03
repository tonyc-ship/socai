"""Browser-level CDP session management."""

from __future__ import annotations

import asyncio
from dataclasses import dataclass
from typing import Any

from .endpoint import Endpoint, resolve_cdp_endpoint
from .page import PageSession


INTERNAL_URL_PREFIXES = (
    "chrome://",
    "chrome-untrusted://",
    "devtools://",
    "chrome-extension://",
    "about:",
)


@dataclass(frozen=True)
class TargetInfo:
    target_id: str
    type: str
    title: str = ""
    url: str = ""
    attached: bool = False

    @classmethod
    def from_cdp(cls, payload: dict[str, Any]) -> "TargetInfo":
        return cls(
            target_id=str(payload.get("targetId") or ""),
            type=str(payload.get("type") or ""),
            title=str(payload.get("title") or ""),
            url=str(payload.get("url") or ""),
            attached=bool(payload.get("attached")),
        )

    @property
    def is_internal(self) -> bool:
        return self.url.startswith(INTERNAL_URL_PREFIXES)

    @property
    def is_real_page(self) -> bool:
        return self.type == "page" and not self.is_internal

    def to_dict(self) -> dict[str, Any]:
        return {
            "targetId": self.target_id,
            "type": self.type,
            "title": self.title,
            "url": self.url,
            "attached": self.attached,
        }


class BrowserSession:
    """Long-lived browser connection with an active page."""

    def __init__(self, client: Any, *, endpoint: Endpoint | None = None, owns_client: bool = False):
        self.client = client
        self.endpoint = endpoint
        self.owns_client = owns_client
        self.active_page: PageSession | None = None

    @classmethod
    async def connect(
        cls,
        *,
        endpoint: Endpoint | None = None,
        browser_ws_url: str | None = None,
        http_url: str | None = None,
        client: Any | None = None,
        start_client: bool = False,
    ) -> "BrowserSession":
        if client is None:
            endpoint = endpoint or resolve_cdp_endpoint(browser_ws_url=browser_ws_url, http_url=http_url)
            client = await connect_cdp_with_retry(endpoint.browser_ws_url)
            return cls(client, endpoint=endpoint, owns_client=True)
        if start_client:
            await client.start()
        return cls(client, endpoint=endpoint, owns_client=start_client)

    async def stop(self) -> None:
        if self.owns_client:
            await self.client.stop()

    async def send(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        return await self.client.send_raw(method, params or {})

    async def list_targets(self) -> list[TargetInfo]:
        result = await self.send("Target.getTargets")
        return [TargetInfo.from_cdp(item) for item in result.get("targetInfos", [])]

    async def list_pages(self, *, include_internal: bool = False) -> list[TargetInfo]:
        pages = [target for target in await self.list_targets() if target.type == "page"]
        if include_internal:
            return pages
        return [target for target in pages if not target.is_internal]

    async def attach_page(self, target_id: str, *, activate: bool = True) -> PageSession:
        if activate:
            try:
                await self.send("Target.activateTarget", {"targetId": target_id})
            except Exception:
                pass
        attached = await self.send("Target.attachToTarget", {"targetId": target_id, "flatten": True})
        page = PageSession(self.client, target_id=target_id, session_id=str(attached["sessionId"]))
        await page.enable_default_domains()
        self.active_page = page
        return page

    async def ensure_page(self) -> PageSession:
        if self.active_page is not None:
            return self.active_page
        pages = await self.list_pages(include_internal=False)
        if pages:
            return await self.attach_page(pages[0].target_id)
        return await self.new_page()

    async def new_page(self, url: str = "about:blank", *, activate: bool = True) -> PageSession:
        # Create blank first, then navigate after attach. This avoids the
        # createTarget(url) race where load polling can see about:blank.
        created = await self.send("Target.createTarget", {"url": "about:blank"})
        page = await self.attach_page(str(created["targetId"]), activate=activate)
        if url != "about:blank":
            await page.navigate(url)
        return page

    async def switch_page(self, target_id: str) -> PageSession:
        return await self.attach_page(target_id, activate=True)

    async def close_page(self, target_id: str) -> bool:
        result = await self.send("Target.closeTarget", {"targetId": target_id})
        if self.active_page and self.active_page.target_id == target_id:
            self.active_page = None
        return bool(result.get("success", True))


async def connect_cdp_with_retry(
    browser_ws_url: str,
    *,
    attempts: int = 4,
    per_attempt_timeout: float = 12.0,
    pause_seconds: float = 1.0,
) -> Any:
    try:
        from cdp_use.client import CDPClient
    except ModuleNotFoundError as exc:
        raise RuntimeError("Missing dependency: cdp-use. Install Socai browser dependencies.") from exc

    last_error: BaseException | None = None
    for attempt in range(1, attempts + 1):
        client = CDPClient(browser_ws_url)
        try:
            await asyncio.wait_for(client.start(), timeout=per_attempt_timeout)
            return client
        except BaseException as exc:
            last_error = exc
            try:
                await asyncio.wait_for(client.stop(), timeout=2)
            except BaseException:
                pass
            if attempt < attempts:
                await asyncio.sleep(pause_seconds)
    raise RuntimeError(
        "CDP connection failed. Ensure the Socai app backend provided a fresh browser_ws_url "
        f"and Chrome remote debugging is approved if prompted. Last error: {last_error}"
    )
