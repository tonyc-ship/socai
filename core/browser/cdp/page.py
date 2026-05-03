"""Page-scoped CDP primitives."""

from __future__ import annotations

import asyncio
import base64
import json
import math
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass(frozen=True)
class RuntimeEvaluation:
    value: Any = None
    type: str | None = None
    subtype: str | None = None
    description: str | None = None


def _decode_unserializable(value: str) -> Any:
    if value == "NaN":
        return math.nan
    if value == "Infinity":
        return math.inf
    if value == "-Infinity":
        return -math.inf
    if value == "-0":
        return -0.0
    if value.endswith("n"):
        return int(value[:-1])
    return value


def _js_snippet(expression: str, limit: int = 160) -> str:
    snippet = expression.strip().replace("\n", "\\n")
    return snippet[: limit - 3] + "..." if len(snippet) > limit else snippet


def _js_exception_description(result: dict[str, Any], details: dict[str, Any] | None) -> str:
    desc = result.get("description")
    exc = details.get("exception") if details else None
    if not desc and isinstance(exc, dict):
        desc = exc.get("description") or exc.get("value") or exc.get("className")
    if not desc and details:
        desc = details.get("text")
    return str(desc or "JavaScript evaluation failed")


def _has_return_statement(expression: str) -> bool:
    i = 0
    n = len(expression)
    state = "code"
    quote = ""
    while i < n:
        ch = expression[i]
        nxt = expression[i + 1] if i + 1 < n else ""
        if state == "code":
            if ch in ("'", '"', "`"):
                state = "string"
                quote = ch
                i += 1
                continue
            if ch == "/" and nxt == "/":
                state = "line_comment"
                i += 2
                continue
            if ch == "/" and nxt == "*":
                state = "block_comment"
                i += 2
                continue
            if expression.startswith("return", i):
                before = expression[i - 1] if i > 0 else ""
                after = expression[i + 6] if i + 6 < n else ""
                if not (before == "_" or before.isalnum()) and not (after == "_" or after.isalnum()):
                    return True
            i += 1
            continue
        if state == "line_comment":
            if ch == "\n":
                state = "code"
            i += 1
            continue
        if state == "block_comment":
            if ch == "*" and nxt == "/":
                state = "code"
                i += 2
                continue
            i += 1
            continue
        if state == "string":
            if ch == "\\":
                i += 2
                continue
            if ch == quote:
                state = "code"
            i += 1
    return False


def _wrap_expression(expression: str) -> str:
    stripped = expression.strip()
    if _has_return_statement(stripped) and not stripped.startswith("("):
        return f"(function(){{{expression}}})()"
    return expression


class PageSession:
    """A target-attached page session."""

    def __init__(self, client: Any, *, target_id: str, session_id: str):
        self.client = client
        self.target_id = target_id
        self.session_id = session_id

    async def send(self, method: str, params: dict[str, Any] | None = None) -> dict[str, Any]:
        return await self.client.send_raw(method, params or {}, session_id=self.session_id)

    async def enable_default_domains(self) -> None:
        for domain in ("Page", "DOM", "Runtime", "Network"):
            try:
                await self.send(f"{domain}.enable")
            except Exception:
                if domain in {"Page", "Runtime"}:
                    raise

    async def navigate(self, url: str, *, wait_until: str = "domcontentloaded", timeout: float = 15.0) -> dict[str, Any]:
        result = await self.send("Page.navigate", {"url": url})
        if wait_until:
            await self.wait_for_load_state(wait_until, timeout=timeout)
        return result

    async def wait_for_load_state(self, state: str = "domcontentloaded", *, timeout: float = 15.0) -> bool:
        target = state.lower()
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                ready = await self.evaluate("document.readyState")
            except Exception:
                ready = ""
            if target in {"domcontentloaded", "interactive"} and ready in {"interactive", "complete"}:
                return True
            if target in {"load", "complete"} and ready == "complete":
                return True
            await asyncio.sleep(0.25)
        return False

    async def evaluate_raw(self, expression: str, *, await_promise: bool = True) -> RuntimeEvaluation:
        result = await self.send(
            "Runtime.evaluate",
            {
                "expression": expression,
                "returnByValue": True,
                "awaitPromise": await_promise,
            },
        )
        payload = result.get("result") or {}
        details = result.get("exceptionDetails")
        if details or payload.get("subtype") == "error":
            desc = _js_exception_description(payload, details)
            if details:
                line = details.get("lineNumber")
                col = details.get("columnNumber")
                loc = f" at line {line}, column {col}" if line is not None and col is not None else ""
            else:
                loc = ""
            raise RuntimeError(f"JavaScript evaluation failed{loc}: {desc}; expression: {_js_snippet(expression)}")

        if "value" in payload:
            value = payload["value"]
        elif "unserializableValue" in payload:
            value = _decode_unserializable(str(payload["unserializableValue"]))
        else:
            value = None

        return RuntimeEvaluation(
            value=value,
            type=payload.get("type"),
            subtype=payload.get("subtype"),
            description=payload.get("description"),
        )

    async def evaluate(self, expression: str, *, await_promise: bool = True) -> Any:
        result = await self.evaluate_raw(_wrap_expression(expression), await_promise=await_promise)
        return result.value

    async def page_info(self) -> dict[str, Any]:
        value = await self.evaluate(
            """
return {
  url: location.href,
  title: document.title,
  w: innerWidth,
  h: innerHeight,
  sx: scrollX,
  sy: scrollY,
  pw: document.documentElement.scrollWidth,
  ph: document.documentElement.scrollHeight,
  readyState: document.readyState
}
"""
        )
        return value if isinstance(value, dict) else {}

    async def click(self, x: float, y: float, *, button: str = "left", clicks: int = 1) -> None:
        params = {"x": float(x), "y": float(y), "button": button, "clickCount": int(clicks)}
        await self.send("Input.dispatchMouseEvent", {**params, "type": "mousePressed"})
        await self.send("Input.dispatchMouseEvent", {**params, "type": "mouseReleased"})

    async def type_text(self, text: str) -> None:
        await self.send("Input.insertText", {"text": text})

    async def press_key(self, key: str, *, modifiers: int = 0) -> None:
        key_map = {
            "Enter": (13, "Enter", "\r"),
            "Tab": (9, "Tab", "\t"),
            "Backspace": (8, "Backspace", ""),
            "Escape": (27, "Escape", ""),
            "Delete": (46, "Delete", ""),
            " ": (32, "Space", " "),
            "ArrowLeft": (37, "ArrowLeft", ""),
            "ArrowUp": (38, "ArrowUp", ""),
            "ArrowRight": (39, "ArrowRight", ""),
            "ArrowDown": (40, "ArrowDown", ""),
            "Home": (36, "Home", ""),
            "End": (35, "End", ""),
            "PageUp": (33, "PageUp", ""),
            "PageDown": (34, "PageDown", ""),
        }
        vk, code, text = key_map.get(key, (ord(key[0]) if len(key) == 1 else 0, key, key if len(key) == 1 else ""))
        base = {
            "key": key,
            "code": code,
            "modifiers": int(modifiers),
            "windowsVirtualKeyCode": vk,
            "nativeVirtualKeyCode": vk,
        }
        await self.send("Input.dispatchKeyEvent", {"type": "keyDown", **base, **({"text": text} if text else {})})
        if text and len(text) == 1:
            await self.send(
                "Input.dispatchKeyEvent",
                {"type": "char", "text": text, **{k: v for k, v in base.items() if k != "text"}},
            )
        await self.send("Input.dispatchKeyEvent", {"type": "keyUp", **base})

    async def scroll(self, *, delta_y: int = 600, delta_x: int = 0, x: float | None = None, y: float | None = None) -> None:
        info = await self.page_info()
        px = float(x if x is not None else max(1, int(info.get("w", 800)) // 2))
        py = float(y if y is not None else max(1, int(info.get("h", 600)) // 2))
        await self.send(
            "Input.dispatchMouseEvent",
            {"type": "mouseWheel", "x": px, "y": py, "deltaX": int(delta_x), "deltaY": int(delta_y)},
        )

    async def screenshot(self, path: str | Path, *, full: bool = False, max_dim: int | None = None) -> str:
        result = await self.send("Page.captureScreenshot", {"format": "png", "captureBeyondViewport": bool(full)})
        data = result.get("data")
        if not data:
            raise RuntimeError("Page.captureScreenshot did not return image data.")
        out = Path(path)
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_bytes(base64.b64decode(data))
        if max_dim:
            from PIL import Image

            img = Image.open(out)
            if max(img.size) > max_dim:
                img.thumbnail((max_dim, max_dim))
                img.save(out)
        return str(out)

    async def detach(self) -> None:
        await self.client.send_raw("Target.detachFromTarget", {"sessionId": self.session_id})
