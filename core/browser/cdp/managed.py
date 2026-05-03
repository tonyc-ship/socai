"""Helpers for reusing the user's existing Chrome CDP endpoint."""

from __future__ import annotations

import json
import os
import platform
import subprocess
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any

from .discovery import Endpoint, endpoint_from_http_url


INSPECT_URL = "chrome://inspect/#remote-debugging"
DEFAULT_DEVTOOLS_PORTS = (9222, 9223)


def _fetch_json(url: str, *, timeout: float = 1.0) -> dict[str, Any] | None:
    try:
        with urllib.request.urlopen(url, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8"))
    except (OSError, urllib.error.URLError, json.JSONDecodeError, TimeoutError):
        return None


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
    version = _fetch_json(url)
    if not version:
        return None
    browser_ws_url = str(version.get("webSocketDebuggerUrl") or "")
    if not browser_ws_url:
        return None
    return Endpoint(
        source=source,
        browser_ws_url=browser_ws_url,
        http_version_url=url,
        version={
            key: version.get(key)
            for key in ("Browser", "Protocol-Version", "User-Agent", "V8-Version")
            if key in version
        },
    )


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
    version_url = f"http://127.0.0.1:{port}/json/version"

    try:
        return endpoint_from_http_url(f"http://127.0.0.1:{port}", source=f"active_port:{profile_root}")
    except RuntimeError as exc:
        cause = exc.__cause__
        if isinstance(cause, urllib.error.HTTPError) and cause.code == 404 and ws_path:
            return Endpoint(source=f"active_port:{profile_root}", browser_ws_url=f"ws://127.0.0.1:{port}{ws_path}")
        _ = version_url
        return None


def discover_existing_chrome_endpoint() -> Endpoint | None:
    """Find a CDP endpoint for an already-open browser profile."""

    if env_ws := os.environ.get("SOCAI_CDP_WS"):
        return Endpoint(source="SOCAI_CDP_WS", browser_ws_url=env_ws)
    if env_http := os.environ.get("SOCAI_CDP_URL"):
        return endpoint_from_http_url(env_http, source="SOCAI_CDP_URL")

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
