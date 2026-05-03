"""CDP endpoint discovery for a user-owned Chrome profile."""

from __future__ import annotations

import json
import os
import platform
import subprocess
import time
import urllib.error
import urllib.request
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Any


INSPECT_URL = "chrome://inspect/#remote-debugging"
DEFAULT_DEVTOOLS_PORTS = (9222, 9223)


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


def _try_fetch_json(url: str, *, timeout: float = 1.0) -> dict[str, Any] | None:
    try:
        return fetch_json(url, timeout=timeout)
    except (OSError, urllib.error.URLError, json.JSONDecodeError, TimeoutError):
        return None


def _endpoint_from_version_payload(version: dict[str, Any], *, source: str, version_url: str) -> Endpoint | None:
    browser_ws_url = str(version.get("webSocketDebuggerUrl") or "")
    if not browser_ws_url:
        return None
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


def endpoint_from_http_url(url: str, *, source: str = "http_url", timeout: float = 5.0) -> Endpoint:
    version_url = f"{url.rstrip('/')}/json/version"
    try:
        version = fetch_json(version_url, timeout=timeout)
    except (OSError, urllib.error.URLError, json.JSONDecodeError, TimeoutError) as exc:
        raise RuntimeError(f"CDP HTTP endpoint is not reachable or did not expose /json/version: {url}") from exc

    endpoint = _endpoint_from_version_payload(version, source=source, version_url=version_url)
    if endpoint is None:
        raise RuntimeError(f"CDP HTTP endpoint did not return webSocketDebuggerUrl: {version_url}")
    return endpoint


def resolve_explicit_endpoint(
    *,
    browser_ws_url: str | None = None,
    http_url: str | None = None,
) -> Endpoint | None:
    """Resolve only explicitly supplied CDP endpoints."""

    if browser_ws_url:
        return Endpoint(source="argument", browser_ws_url=browser_ws_url)
    if http_url:
        return endpoint_from_http_url(http_url, source="argument")

    if env_ws := os.environ.get("SOCAI_CDP_WS"):
        return Endpoint(source="SOCAI_CDP_WS", browser_ws_url=env_ws)
    if env_http := os.environ.get("SOCAI_CDP_URL"):
        return endpoint_from_http_url(env_http, source="SOCAI_CDP_URL")

    return None


def resolve_cdp_endpoint(
    *,
    browser_ws_url: str | None = None,
    http_url: str | None = None,
) -> Endpoint:
    endpoint = resolve_explicit_endpoint(browser_ws_url=browser_ws_url, http_url=http_url)
    if endpoint is None:
        raise RuntimeError(
            "No CDP endpoint was provided. Pass browser_ws_url/Endpoint from the Socai app backend, "
            "or set SOCAI_CDP_WS / SOCAI_CDP_URL for local development."
        )
    return endpoint


def chrome_profile_roots() -> list[Path]:
    """Return profile roots where an already-open Chrome may expose CDP state."""

    if override := os.environ.get("SOCAI_CHROME_USER_DATA_DIR"):
        return [Path(override).expanduser()]

    home = Path.home()
    system = platform.system()
    if system == "Darwin":
        paths = [
            home / "Library/Application Support/Google/Chrome",
            home / "Library/Application Support/Comet",
            home / "Library/Application Support/Arc/User Data",
            home / "Library/Application Support/Microsoft Edge",
            home / "Library/Application Support/BraveSoftware/Brave-Browser",
        ]
    elif system == "Windows":
        paths = [
            home / "AppData/Local/Google/Chrome/User Data",
            home / "AppData/Local/Microsoft/Edge/User Data",
            home / "AppData/Local/BraveSoftware/Brave-Browser/User Data",
        ]
    else:
        paths = [
            home / ".config/google-chrome",
            home / ".config/chromium",
            home / ".config/microsoft-edge",
            home / ".config/BraveSoftware/Brave-Browser",
        ]

    seen: set[str] = set()
    result: list[Path] = []
    for path in paths:
        key = str(path)
        if key not in seen:
            seen.add(key)
            result.append(path)
    return result


def _endpoint_from_version_url(url: str, *, source: str) -> Endpoint | None:
    version = _try_fetch_json(url)
    if not version:
        return None
    return _endpoint_from_version_payload(version, source=source, version_url=url)


def _endpoint_from_active_port(profile_root: Path) -> Endpoint | None:
    marker = profile_root / "DevToolsActivePort"
    try:
        lines = marker.read_text(encoding="utf-8").splitlines()
    except (FileNotFoundError, NotADirectoryError, OSError):
        return None
    if not lines:
        return None

    try:
        port = int(lines[0].strip())
    except ValueError:
        return None
    ws_path = lines[1].strip() if len(lines) > 1 else ""

    try:
        return endpoint_from_http_url(f"http://127.0.0.1:{port}", source=f"active_port:{profile_root}")
    except RuntimeError as exc:
        cause = exc.__cause__
        if isinstance(cause, urllib.error.HTTPError) and cause.code == 404 and ws_path:
            return Endpoint(source=f"active_port:{profile_root}", browser_ws_url=f"ws://127.0.0.1:{port}{ws_path}")
        return None


def discover_existing_chrome_endpoint() -> Endpoint | None:
    """Find a CDP endpoint for an already-open browser profile."""

    explicit = resolve_explicit_endpoint()
    if explicit is not None:
        return explicit

    for profile_root in chrome_profile_roots():
        endpoint = _endpoint_from_active_port(profile_root)
        if endpoint is not None:
            return endpoint

    for port in DEFAULT_DEVTOOLS_PORTS:
        endpoint = _endpoint_from_version_url(f"http://127.0.0.1:{port}/json/version", source=f"port:{port}")
        if endpoint is not None:
            return endpoint
    return None


def wait_for_existing_chrome_endpoint(*, timeout: float = 45.0, poll_seconds: float = 0.5) -> Endpoint | None:
    deadline = time.time() + max(0.1, timeout)
    while time.time() < deadline:
        endpoint = discover_existing_chrome_endpoint()
        if endpoint is not None:
            return endpoint
        time.sleep(max(0.1, poll_seconds))
    return None


def open_remote_debugging_page() -> None:
    if platform.system() == "Darwin":
        subprocess.run(["open", "-a", "Google Chrome", INSPECT_URL], check=False)
        return

    import webbrowser

    webbrowser.open(INSPECT_URL, new=2)


discover_chrome_cdp = resolve_cdp_endpoint
