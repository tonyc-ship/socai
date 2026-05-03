"""Chrome DevTools Protocol runtime primitives."""

from .browser import BrowserSession, TargetInfo, connect_cdp_with_retry
from .discovery import Endpoint, discover_chrome_cdp, endpoint_from_http_url, resolve_cdp_endpoint
from .managed import (
    discover_existing_chrome_endpoint,
    open_remote_debugging_page,
    wait_for_existing_chrome_endpoint,
)
from .page import PageSession, RuntimeEvaluation

__all__ = [
    "BrowserSession",
    "Endpoint",
    "PageSession",
    "RuntimeEvaluation",
    "TargetInfo",
    "connect_cdp_with_retry",
    "discover_chrome_cdp",
    "discover_existing_chrome_endpoint",
    "endpoint_from_http_url",
    "open_remote_debugging_page",
    "resolve_cdp_endpoint",
    "wait_for_existing_chrome_endpoint",
]
