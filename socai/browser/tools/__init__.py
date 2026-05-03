"""Agent tools for generic browser control."""

from .browser import (
    BrowserClickTool,
    BrowserEvalTool,
    BrowserListTabsTool,
    BrowserNavigateTool,
    BrowserNewTabTool,
    BrowserPageInfoTool,
    BrowserPressKeyTool,
    BrowserScreenshotTool,
    BrowserScrollTool,
    BrowserSwitchTabTool,
    BrowserTypeTool,
    build_browser_tools,
)

__all__ = [
    "BrowserClickTool",
    "BrowserEvalTool",
    "BrowserListTabsTool",
    "BrowserNavigateTool",
    "BrowserNewTabTool",
    "BrowserPageInfoTool",
    "BrowserPressKeyTool",
    "BrowserScreenshotTool",
    "BrowserScrollTool",
    "BrowserSwitchTabTool",
    "BrowserTypeTool",
    "build_browser_tools",
]
