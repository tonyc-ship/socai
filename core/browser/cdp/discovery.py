"""Chrome CDP endpoint discovery.

Socai connects to an already-running browser profile. This module only finds
the DevTools endpoint; it does not start Chrome.
"""

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
COMMON_DEVTOOLS_PORTS = (9222, 9223)


@dataclass(frozen=True)
class Endpoint:
    source: str
    browser_ws_url: str
    port: int | None = None
    http_version_url: str | None = None
    user_data_dir: str | None = None
    version: dict[str, Any] | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


def chrome_user_data_dirs() -> list[Path]:
    """Return candidate Chromium-family profile roots."""

    candidates: list[Path] = []
    if override := os.environ.get("SOCAI_CHROME_USER_DATA_DIR"):
        candidates.append(Path(override).expanduser())
        if os.environ.get("SOCAI_CHROME_USER_DATA_DIR_ONLY") == "1":
            return candidates

    home = Path.home()
    system = platform.system()
    if system == "Darwin":
        candidates.extend(
            [
                home / "Library/Application Support/Google/Chrome",
                home / "Library/Application Support/Microsoft Edge",
                home / "Library/Application Support/BraveSoftware/Brave-Browser",
                home / "Library/Application Support/Arc/User Data",
            ]
        )
    elif system == "Windows":
        candidates.extend(
            [
                home / "AppData/Local/Google/Chrome/User Data",
                home / "AppData/Local/Microsoft/Edge/User Data",
                home / "AppData/Local/BraveSoftware/Brave-Browser/User Data",
            ]
        )
    else:
        candidates.extend(
            [
                home / ".config/google-chrome",
                home / ".config/chromium",
                home / ".config/microsoft-edge",
                home / ".config/BraveSoftware/Brave-Browser",
            ]
        )

    seen: set[str] = set()
    out: list[Path] = []
    for path in candidates:
        key = str(path)
        if key in seen:
            continue
        seen.add(key)
        out.append(path)
    return out


def fetch_json(url: str, *, timeout: float = 1.5) -> dict[str, Any]:
    with urllib.request.urlopen(url, timeout=timeout) as response:
        return json.loads(response.read().decode("utf-8"))


def _endpoint_from_http_url(url: str, *, source: str, timeout: float = 1.5) -> Endpoint | None:
    base = url.rstrip("/")
    version_url = f"{base}/json/version"
    try:
        version = fetch_json(version_url, timeout=timeout)
    except (OSError, urllib.error.URLError, json.JSONDecodeError, TimeoutError):
        return None

    browser_ws_url = version.get("webSocketDebuggerUrl")
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


def _endpoint_from_port(port: int, *, source: str, timeout: float = 1.5) -> Endpoint | None:
    endpoint = _endpoint_from_http_url(f"http://127.0.0.1:{port}", source=source, timeout=timeout)
    if endpoint is None:
        return None
    return Endpoint(
        source=endpoint.source,
        browser_ws_url=endpoint.browser_ws_url,
        port=port,
        http_version_url=endpoint.http_version_url,
        user_data_dir=endpoint.user_data_dir,
        version=endpoint.version,
    )


def _endpoint_from_active_port(user_data_dir: Path, *, timeout: float = 1.5) -> Endpoint | None:
    marker = user_data_dir / "DevToolsActivePort"
    try:
        lines = marker.read_text(encoding="utf-8").splitlines()
    except (FileNotFoundError, OSError):
        return None
    if not lines:
        return None

    try:
        port = int(lines[0].strip())
    except ValueError:
        return None

    deadline = time.time() + timeout
    while time.time() < deadline:
        endpoint = _endpoint_from_port(port, source="devtools_active_port", timeout=0.5)
        if endpoint is not None:
            return Endpoint(
                source=endpoint.source,
                browser_ws_url=endpoint.browser_ws_url,
                port=port,
                http_version_url=endpoint.http_version_url,
                user_data_dir=str(user_data_dir),
                version=endpoint.version,
            )
        time.sleep(0.1)

    # Some Chrome versions can 404 /json/version for default profiles while
    # the ws path in DevToolsActivePort is still usable.
    if len(lines) > 1 and lines[1].strip():
        return Endpoint(
            source="devtools_active_port",
            browser_ws_url=f"ws://127.0.0.1:{port}{lines[1].strip()}",
            port=port,
            user_data_dir=str(user_data_dir),
        )
    return None


def discover_chrome_cdp() -> Endpoint:
    """Return a live Chrome CDP endpoint or raise a setup-oriented error."""

    if ws_url := os.environ.get("SOCAI_CDP_WS"):
        return Endpoint(source="SOCAI_CDP_WS", browser_ws_url=ws_url)

    if http_url := os.environ.get("SOCAI_CDP_URL"):
        endpoint = _endpoint_from_http_url(http_url, source="SOCAI_CDP_URL", timeout=5.0)
        if endpoint is not None:
            return endpoint
        raise RuntimeError(f"SOCAI_CDP_URL={http_url} is not reachable or did not expose /json/version.")

    for user_data_dir in chrome_user_data_dirs():
        endpoint = _endpoint_from_active_port(user_data_dir, timeout=1.5)
        if endpoint is not None:
            return endpoint

    for port in COMMON_DEVTOOLS_PORTS:
        endpoint = _endpoint_from_port(port, source="common_devtools_port", timeout=0.8)
        if endpoint is not None:
            return endpoint

    checked = ", ".join(str(path) for path in chrome_user_data_dirs())
    raise RuntimeError(
        "No live Chrome CDP endpoint found. Open Chrome, visit "
        f"{INSPECT_URL}, approve remote debugging if prompted, then retry. "
        f"Checked: {checked}"
    )


def open_inspect_page() -> None:
    if platform.system() == "Darwin":
        subprocess.run(["open", "-a", "Google Chrome", INSPECT_URL], check=False)
        return

    import webbrowser

    webbrowser.open(INSPECT_URL, new=2)
