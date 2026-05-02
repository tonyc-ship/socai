"""CDP transport boundary.

Only this module knows about ``cdp-use``. Higher layers depend on the small
``CdpTransport`` protocol.
"""

from __future__ import annotations

import asyncio
from typing import Any, Protocol


class CdpTransport(Protocol):
    async def start(self) -> None: ...

    async def stop(self) -> None: ...

    async def send(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        session_id: str | None = None,
    ) -> dict[str, Any]: ...


class CdpUseTransport:
    """Transport adapter backed by ``cdp_use.client.CDPClient``."""

    def __init__(self, browser_ws_url: str):
        self.browser_ws_url = browser_ws_url
        self._client: Any | None = None

    async def start(self) -> None:
        try:
            from cdp_use.client import CDPClient
        except ModuleNotFoundError as exc:
            raise RuntimeError("Missing dependency: cdp-use. Install Socai browser dependencies.") from exc

        self._client = CDPClient(self.browser_ws_url)
        await self._client.start()

    async def stop(self) -> None:
        if self._client is None:
            return
        await self._client.stop()
        self._client = None

    async def send(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        session_id: str | None = None,
    ) -> dict[str, Any]:
        if self._client is None:
            raise RuntimeError("CDP transport has not been started.")
        return await self._client.send_raw(method, params or {}, session_id=session_id)


async def connect_with_retry(
    transport: CdpTransport,
    *,
    attempts: int = 4,
    per_attempt_timeout: float = 12.0,
    pause_seconds: float = 1.0,
) -> CdpTransport:
    last_error: BaseException | None = None
    for attempt in range(1, attempts + 1):
        try:
            await asyncio.wait_for(transport.start(), timeout=per_attempt_timeout)
            return transport
        except BaseException as exc:
            last_error = exc
            try:
                await asyncio.wait_for(transport.stop(), timeout=2)
            except BaseException:
                pass
            if attempt < attempts:
                await asyncio.sleep(pause_seconds)
    raise RuntimeError(
        "CDP connection failed. Open chrome://inspect/#remote-debugging, approve "
        f"Chrome remote-debugging permission if prompted, then retry. Last error: {last_error}"
    )
