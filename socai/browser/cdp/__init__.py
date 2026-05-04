"""Chrome DevTools Protocol runtime primitives."""

from .browser import BrowserSession, TargetInfo, connect_cdp_with_retry
from .endpoint import (
    Endpoint,
    discover_chrome_cdp,
    discover_existing_chrome_endpoint,
    endpoint_from_http_url,
    open_remote_debugging_page,
    resolve_cdp_endpoint,
    resolve_explicit_endpoint,
    wait_for_existing_chrome_endpoint,
)
from .page import PageGoneError, PageSession, RuntimeEvaluation
from .task_session import BrowserTaskSession, BrowserTaskSessionManager

__all__ = [
    "BrowserSession",
    "BrowserTaskSession",
    "BrowserTaskSessionManager",
    "Endpoint",
    "PageGoneError",
    "PageSession",
    "RuntimeEvaluation",
    "TargetInfo",
    "connect_cdp_with_retry",
    "discover_chrome_cdp",
    "discover_existing_chrome_endpoint",
    "endpoint_from_http_url",
    "open_remote_debugging_page",
    "resolve_cdp_endpoint",
    "resolve_explicit_endpoint",
    "wait_for_existing_chrome_endpoint",
]
