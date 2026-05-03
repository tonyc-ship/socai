"""Explicit CDP endpoint resolution.

The Socai app/backend owns browser discovery and permission handling. This
module only normalizes a CDP websocket URL that was passed in directly or
provided through environment variables.
"""

from __future__ import annotations

import json
import os
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass
from typing import Any


@dataclass(frozen=True)
class Endpoint:
    source: str
    browser_ws_url: str
    http_version_url: str | None = None
    version: dict[str, Any] | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


def fetch_json(url: str, *, timeout: float = 5.0) -> dict[str, Any]:
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def endpoint_from_http_url(url: str, *, source: str = "http_url", timeout: float = 5.0) -> Endpoint:
    version_url = f"{url.rstrip('/')}/json/version"
    try:
        version = fetch_json(version_url, timeout=timeout)
    except (OSError, urllib.error.URLError, json.JSONDecodeError, TimeoutError) as exc:
        raise RuntimeError(f"CDP HTTP endpoint is not reachable or did not expose /json/version: {url}") from exc

    browser_ws_url = str(version.get("webSocketDebuggerUrl") or "")
    if not browser_ws_url:
        raise RuntimeError(f"CDP HTTP endpoint did not return webSocketDebuggerUrl: {version_url}")

    return Endpoint(
        source=source,
        browser_ws_url=browser_ws_url,
        http_version_url=version_url,
        version={
            key: version.get(key)
            for key in ("Browser", "Protocol-Version", "User-Agent", "V8-Version")
            if key in version
        },
    )


def resolve_cdp_endpoint(
    *,
    browser_ws_url: str | None = None,
    http_url: str | None = None,
) -> Endpoint:
    """Resolve an explicit CDP endpoint.

    Priority:
    1. function arguments from the app/backend
    2. ``SOCAI_CDP_WS``
    3. ``SOCAI_CDP_URL`` pointing at an HTTP DevTools endpoint
    """

    if browser_ws_url:
        return Endpoint(source="argument", browser_ws_url=browser_ws_url)
    if http_url:
        return endpoint_from_http_url(http_url, source="argument")

    if env_ws := os.environ.get("SOCAI_CDP_WS"):
        return Endpoint(source="SOCAI_CDP_WS", browser_ws_url=env_ws)
    if env_http := os.environ.get("SOCAI_CDP_URL"):
        return endpoint_from_http_url(env_http, source="SOCAI_CDP_URL")

    raise RuntimeError(
        "No CDP endpoint was provided. Pass browser_ws_url/Endpoint from the Socai app backend, "
        "or set SOCAI_CDP_WS / SOCAI_CDP_URL for local development."
    )


discover_chrome_cdp = resolve_cdp_endpoint
