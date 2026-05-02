from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

from core.agent import ToolContext
from core.browser.tools import BrowserEvalTool, build_browser_tools


class FakeBrowser:
    def __init__(self) -> None:
        self.page = FakePage()

    async def ensure_page(self):
        return self.page


class FakePage:
    async def evaluate(self, expression: str):
        return {"expression": expression, "answer": 42}


class BrowserToolTests(unittest.IsolatedAsyncioTestCase):
    async def test_browser_eval_tool_returns_json_value(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tool = BrowserEvalTool(FakeBrowser())

            raw = await tool.execute({"code": "return 42"}, ToolContext(run_dir=Path(tmp)))

            payload = json.loads(raw)
            self.assertTrue(payload["ok"])
            self.assertEqual(payload["value"]["answer"], 42)
            self.assertEqual(payload["value"]["expression"], "return 42")

    def test_build_browser_tools_keeps_minimal_core_surface(self) -> None:
        names = [tool.name for tool in build_browser_tools(FakeBrowser())]

        self.assertEqual(
            names,
            [
                "browser_new_tab",
                "browser_navigate",
                "browser_page_info",
                "browser_screenshot",
                "browser_click",
                "browser_type",
                "browser_press_key",
                "browser_scroll",
                "browser_eval_js",
                "browser_list_tabs",
                "browser_switch_tab",
            ],
        )


if __name__ == "__main__":
    unittest.main()
