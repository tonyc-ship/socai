"""Chrome DevTools Protocol runtime primitives."""

from .browser import BrowserSession, TargetInfo
from .discovery import Endpoint, discover_chrome_cdp, open_inspect_page
from .page import PageSession, RuntimeEvaluation
from .transport import CdpTransport, CdpUseTransport

__all__ = [
    "BrowserSession",
    "CdpTransport",
    "CdpUseTransport",
    "Endpoint",
    "PageSession",
    "RuntimeEvaluation",
    "TargetInfo",
    "discover_chrome_cdp",
    "open_inspect_page",
]
