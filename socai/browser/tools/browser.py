"""Generic browser tools for the agent loop."""

from __future__ import annotations

import json
from functools import lru_cache
from pathlib import Path

from socai.agent.tool import Tool, ToolContext
from socai.browser.cdp.browser import BrowserSession


def _json(payload: dict) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2)


BROWSER_PAGE_SCRIPTS_JS = Path(__file__).with_name("page_scripts.js")
BROWSER_PAGE_SCRIPT_FUNCTIONS = {"clickSelector", "fillSelector", "waitForSelector"}


@lru_cache(maxsize=1)
def load_browser_page_scripts() -> str:
    return BROWSER_PAGE_SCRIPTS_JS.read_text(encoding="utf-8")


def browser_page_script_call(name: str, arg: dict) -> str:
    if name not in BROWSER_PAGE_SCRIPT_FUNCTIONS:
        raise ValueError(f"Unknown browser page script: {name}")
    args = json.dumps(arg, ensure_ascii=False)
    return f"{load_browser_page_scripts()}\n// SOCAI_BROWSER_CALL: {name}\nreturn SocaiBrowserPageScripts.{name}({args});"


class BrowserToolBase(Tool):
    def __init__(self, browser: BrowserSession):
        self.browser = browser

    async def _page(self):
        return await self.browser.ensure_page()


class BrowserNewTabTool(BrowserToolBase):
    name = "browser_new_tab"
    description = "Open a new browser tab and optionally navigate it to a URL. Returns target/page info."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "url": {"type": "string", "description": "URL to open. Defaults to about:blank."},
                "activate": {"type": "boolean", "default": True},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self.browser.new_page(str(params.get("url") or "about:blank"), activate=params.get("activate", True))
        return _json({"targetId": page.target_id, "sessionId": page.session_id, "page": await page.page_info()})


class BrowserNavigateTool(BrowserToolBase):
    name = "browser_navigate"
    description = "Navigate the active tab to a URL and wait for DOM readiness."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "url": {"type": "string"},
                "wait_until": {
                    "type": "string",
                    "enum": ["domcontentloaded", "load", "complete", "none"],
                    "default": "domcontentloaded",
                },
                "timeout": {"type": "number", "default": 15},
            },
            "required": ["url"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        wait_until = str(params.get("wait_until") or "domcontentloaded")
        await page.navigate(
            str(params["url"]),
            wait_until="" if wait_until == "none" else wait_until,
            timeout=float(params.get("timeout", 15)),
        )
        return _json({"ok": True, "page": await page.page_info()})


class BrowserPageInfoTool(BrowserToolBase):
    name = "browser_page_info"
    description = "Return active page URL, title, viewport, scroll position, and ready state."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        return _json(await page.page_info())


class BrowserScreenshotTool(BrowserToolBase):
    name = "browser_screenshot"
    description = "Capture a PNG screenshot of the active browser tab and save it as an artifact."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "label": {"type": "string", "default": "browser"},
                "full": {"type": "boolean", "default": False},
                "max_dim": {"type": "integer", "description": "Optional maximum image dimension."},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        label = str(params.get("label") or "browser")
        path = ctx.next_screenshot_path(label)
        saved = await page.screenshot(
            path,
            full=bool(params.get("full", False)),
            max_dim=int(params["max_dim"]) if params.get("max_dim") else None,
        )
        rel = ctx.register_artifact(
            Path(saved),
            label=label,
            artifact_kind="image",
            summary=f"Browser screenshot: {label}",
            metadata={"category": "screenshot"},
            source_tool=self.name,
        )
        return f"Screenshot saved to {rel}"


class BrowserClickTool(BrowserToolBase):
    name = "browser_click"
    description = "Click viewport coordinates in the active tab using CDP mouse input."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "x": {"type": "number"},
                "y": {"type": "number"},
                "button": {"type": "string", "default": "left"},
                "clicks": {"type": "integer", "default": 1},
            },
            "required": ["x", "y"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        await page.click(float(params["x"]), float(params["y"]), button=str(params.get("button") or "left"), clicks=int(params.get("clicks", 1)))
        return _json({"ok": True, "x": params["x"], "y": params["y"]})


class BrowserClickSelectorTool(BrowserToolBase):
    name = "browser_click_selector"
    description = (
        "Find a CSS selector in the active tab, scroll it into view, and click it via CDP mouse "
        "events at the element center. Returns the click coordinates."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "selector": {"type": "string"},
                "button": {"type": "string", "default": "left"},
                "clicks": {"type": "integer", "default": 1},
            },
            "required": ["selector"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        selector = str(params["selector"])
        info = await page.evaluate(browser_page_script_call("clickSelector", {"selector": selector}))
        if not isinstance(info, dict) or not info.get("ok"):
            return _json({"ok": False, "selector": selector, "error": (info or {}).get("error", "unknown")})
        await page.click(
            float(info["x"]),
            float(info["y"]),
            button=str(params.get("button") or "left"),
            clicks=int(params.get("clicks", 1)),
        )
        return _json({"ok": True, "selector": selector, "x": info["x"], "y": info["y"]})


class BrowserFillTool(BrowserToolBase):
    name = "browser_fill"
    description = (
        "Find a CSS selector in the active tab, focus it, clear existing value, and type the given text. "
        "Use this instead of browser_type when you know the target selector."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "selector": {"type": "string"},
                "text": {"type": "string"},
                "press_enter": {"type": "boolean", "default": False},
            },
            "required": ["selector", "text"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        selector = str(params["selector"])
        text = str(params.get("text") or "")
        info = await page.evaluate(browser_page_script_call("fillSelector", {"selector": selector}))
        if not isinstance(info, dict) or not info.get("ok"):
            return _json({"ok": False, "selector": selector, "error": (info or {}).get("error", "unknown")})
        # Click at element center as belt-and-suspenders so framework inputs receive focus.
        await page.click(float(info["x"]), float(info["y"]))
        await page.type_text(text)
        if bool(params.get("press_enter")):
            await page.press_key("Enter")
        return _json({"ok": True, "selector": selector, "length": len(text)})


class BrowserWaitForSelectorTool(BrowserToolBase):
    name = "browser_wait_for_selector"
    description = (
        "Wait for a CSS selector to appear in the active tab (optionally visible), up to timeout. "
        "Uses an in-page MutationObserver so the wait resolves the moment the element appears — no polling."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "selector": {"type": "string"},
                "timeout": {"type": "number", "default": 8.0},
                "require_visible": {"type": "boolean", "default": True},
            },
            "required": ["selector"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        selector = str(params["selector"])
        timeout = max(0.1, float(params.get("timeout", 8.0)))
        require_visible = bool(params.get("require_visible", True))

        # Single round-trip: a MutationObserver inside the page resolves as soon
        # as the selector appears, or after timeout. evaluate(awaitPromise=True)
        # blocks Python until that Promise settles, so we don't poll over CDP.
        value = await page.evaluate(
            browser_page_script_call(
                "waitForSelector",
                {"selector": selector, "timeout_ms": int(timeout * 1000), "require_visible": require_visible},
            )
        )
        if isinstance(value, dict) and value.get("found"):
            return _json({"ok": True, "selector": selector, "state": value})
        return _json({"ok": False, "selector": selector, "state": value or {"found": False}, "error": "timeout"})


class BrowserTypeTool(BrowserToolBase):
    name = "browser_type"
    description = "Insert text at the focused element in the active tab."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {"text": {"type": "string"}},
            "required": ["text"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        text = str(params.get("text") or "")
        await page.type_text(text)
        return _json({"ok": True, "length": len(text)})


class BrowserPressKeyTool(BrowserToolBase):
    name = "browser_press_key"
    description = "Press a key in the active tab using CDP keyboard input."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "key": {"type": "string", "description": "Examples: Enter, Escape, Backspace, ArrowDown, a."},
                "modifiers": {"type": "integer", "description": "Bitfield: 1=Alt, 2=Ctrl, 4=Meta, 8=Shift.", "default": 0},
            },
            "required": ["key"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        await page.press_key(str(params["key"]), modifiers=int(params.get("modifiers", 0)))
        return _json({"ok": True, "key": params["key"]})


class BrowserScrollTool(BrowserToolBase):
    name = "browser_scroll"
    description = "Scroll the active tab with a CDP mouse wheel event."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "delta_y": {"type": "integer", "default": 600},
                "delta_x": {"type": "integer", "default": 0},
                "x": {"type": "number"},
                "y": {"type": "number"},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        await page.scroll(
            delta_y=int(params.get("delta_y", 600)),
            delta_x=int(params.get("delta_x", 0)),
            x=params.get("x"),
            y=params.get("y"),
        )
        return _json({"ok": True})


class BrowserEvalTool(BrowserToolBase):
    name = "browser_eval_js"
    description = "Evaluate JavaScript in the active tab. Use return statements for multi-line code."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {"code": {"type": "string"}},
            "required": ["code"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self._page()
        value = await page.evaluate(str(params.get("code") or "undefined"))
        return _json({"ok": True, "value": value})


class BrowserListTabsTool(BrowserToolBase):
    name = "browser_list_tabs"
    description = "List browser page targets. Internal chrome:// pages are hidden by default."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {"include_internal": {"type": "boolean", "default": False}},
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        pages = await self.browser.list_pages(include_internal=bool(params.get("include_internal", False)))
        return _json({"tabs": [page.to_dict() for page in pages], "count": len(pages)})


class BrowserSwitchTabTool(BrowserToolBase):
    name = "browser_switch_tab"
    description = "Attach Socai to an existing tab by CDP targetId and activate it."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {"target_id": {"type": "string"}},
            "required": ["target_id"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        page = await self.browser.switch_page(str(params["target_id"]))
        return _json({"ok": True, "targetId": page.target_id, "page": await page.page_info()})


def build_browser_tools(browser: BrowserSession) -> list[Tool]:
    return [
        BrowserNewTabTool(browser),
        BrowserNavigateTool(browser),
        BrowserPageInfoTool(browser),
        BrowserScreenshotTool(browser),
        BrowserClickTool(browser),
        BrowserClickSelectorTool(browser),
        BrowserTypeTool(browser),
        BrowserFillTool(browser),
        BrowserPressKeyTool(browser),
        BrowserScrollTool(browser),
        BrowserWaitForSelectorTool(browser),
        BrowserEvalTool(browser),
        BrowserListTabsTool(browser),
        BrowserSwitchTabTool(browser),
    ]
