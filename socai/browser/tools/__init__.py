"""Agent tools for generic browser control."""

from .browser import (
    BrowserClickSelectorTool,
    BrowserClickTool,
    BrowserEvalTool,
    BrowserFillTool,
    BrowserListTabsTool,
    BrowserNavigateTool,
    BrowserNewTabTool,
    BrowserPageInfoTool,
    BrowserPressKeyTool,
    BrowserScreenshotTool,
    BrowserScrollTool,
    BrowserSwitchTabTool,
    BrowserTypeTool,
    BrowserWaitForSelectorTool,
    build_browser_tools,
)

__all__ = [
    "BrowserClickSelectorTool",
    "BrowserClickTool",
    "BrowserEvalTool",
    "BrowserFillTool",
    "BrowserListTabsTool",
    "BrowserNavigateTool",
    "BrowserNewTabTool",
    "BrowserPageInfoTool",
    "BrowserPressKeyTool",
    "BrowserScreenshotTool",
    "BrowserScrollTool",
    "BrowserSwitchTabTool",
    "BrowserTypeTool",
    "BrowserWaitForSelectorTool",
    "build_browser_tools",
]
