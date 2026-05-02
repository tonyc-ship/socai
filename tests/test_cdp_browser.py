from __future__ import annotations

import unittest
from typing import Any

from core.browser.cdp import BrowserSession, PageSession


class FakeTransport:
    def __init__(self) -> None:
        self.calls: list[tuple[str, dict[str, Any], str | None]] = []
        self.started = False
        self.stopped = False

    async def start(self) -> None:
        self.started = True

    async def stop(self) -> None:
        self.stopped = True

    async def send(
        self,
        method: str,
        params: dict[str, Any] | None = None,
        *,
        session_id: str | None = None,
    ) -> dict[str, Any]:
        payload = params or {}
        self.calls.append((method, payload, session_id))

        if method == "Target.createTarget":
            return {"targetId": "target-1"}
        if method == "Target.activateTarget":
            return {}
        if method == "Target.attachToTarget":
            return {"sessionId": "session-1"}
        if method == "Target.getTargets":
            return {
                "targetInfos": [
                    {"targetId": "target-1", "type": "page", "title": "Real", "url": "https://example.test"},
                    {"targetId": "target-2", "type": "page", "title": "Internal", "url": "chrome://settings"},
                ]
            }
        if method.endswith(".enable"):
            return {}
        if method == "Page.navigate":
            return {"frameId": "frame-1"}
        if method == "Runtime.evaluate":
            expression = str(payload.get("expression") or "")
            if expression == "document.readyState":
                return {"result": {"type": "string", "value": "complete"}}
            if "document.title" in expression:
                return {
                    "result": {
                        "type": "object",
                        "value": {
                            "url": "https://example.test",
                            "title": "Example",
                            "w": 1024,
                            "h": 768,
                            "sx": 0,
                            "sy": 0,
                            "pw": 1024,
                            "ph": 1800,
                            "readyState": "complete",
                        },
                    }
                }
            return {"result": {"type": "number", "value": 1}}
        if method == "Input.dispatchKeyEvent":
            return {}
        raise AssertionError(f"Unhandled CDP method: {method}")


class CdpBrowserTests(unittest.IsolatedAsyncioTestCase):
    async def test_connect_with_injected_transport_skips_endpoint_discovery(self) -> None:
        transport = FakeTransport()

        browser = await BrowserSession.connect(transport=transport)

        self.assertTrue(transport.started)
        self.assertIsNone(browser.endpoint)

    async def test_new_page_attaches_blank_then_navigates(self) -> None:
        transport = FakeTransport()
        browser = BrowserSession(transport)

        page = await browser.new_page("https://example.test")

        self.assertEqual(page.target_id, "target-1")
        self.assertEqual(page.session_id, "session-1")
        self.assertIs(browser.active_page, page)
        self.assertEqual(transport.calls[0], ("Target.createTarget", {"url": "about:blank"}, None))
        self.assertIn(("Page.navigate", {"url": "https://example.test"}, "session-1"), transport.calls)
        enabled = [method for method, _, session_id in transport.calls if method.endswith(".enable") and session_id == "session-1"]
        self.assertEqual(enabled, ["Page.enable", "DOM.enable", "Runtime.enable", "Network.enable"])

    async def test_list_pages_hides_internal_tabs_by_default(self) -> None:
        transport = FakeTransport()
        browser = BrowserSession(transport)

        pages = await browser.list_pages()
        all_pages = await browser.list_pages(include_internal=True)

        self.assertEqual([page.target_id for page in pages], ["target-1"])
        self.assertEqual([page.target_id for page in all_pages], ["target-1", "target-2"])

    async def test_page_evaluate_wraps_return_statement_and_decodes_value(self) -> None:
        transport = FakeTransport()
        page = PageSession(transport, target_id="target-1", session_id="session-1")

        value = await page.evaluate("return 1")

        self.assertEqual(value, 1)
        evaluate_call = transport.calls[-1]
        self.assertEqual(evaluate_call[0], "Runtime.evaluate")
        self.assertEqual(evaluate_call[1]["expression"], "(function(){return 1})()")
        self.assertEqual(evaluate_call[2], "session-1")

    async def test_press_key_dispatches_key_char_and_keyup(self) -> None:
        transport = FakeTransport()
        page = PageSession(transport, target_id="target-1", session_id="session-1")

        await page.press_key("a")

        events = [params["type"] for method, params, _ in transport.calls if method == "Input.dispatchKeyEvent"]
        self.assertEqual(events, ["keyDown", "char", "keyUp"])


if __name__ == "__main__":
    unittest.main()
