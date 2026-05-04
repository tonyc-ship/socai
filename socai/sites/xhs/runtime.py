"""Minimal Xiaohongshu runtime on top of CDP PageSession."""

from __future__ import annotations

import asyncio
import json
import re
from functools import lru_cache
from pathlib import Path
from typing import Any

from socai.browser.cdp.page import PageSession

from .entities import XhsNote, XhsNoteCard


XHS_HOME_URL = "https://www.xiaohongshu.com/explore"
XHS_PAGE_SCRIPTS_JS = Path(__file__).with_name("page_scripts.js")
XHS_PAGE_SCRIPT_FUNCTIONS = {
    "note",
    "noteWithWait",
    "searchCards",
    "searchInput",
    "searchState",
    "searchTabs",
    "clickSearchTab",
    "clickCard",
    "closeNote",
    "noteOpen",
    "comments",
    "scrollInNote",
}


@lru_cache(maxsize=1)
def load_xhs_page_scripts() -> str:
    return XHS_PAGE_SCRIPTS_JS.read_text(encoding="utf-8")


def xhs_page_script_call(name: str, arg: Any = None) -> str:
    if name not in XHS_PAGE_SCRIPT_FUNCTIONS:
        raise ValueError(f"Unknown XHS page script: {name}")
    args = "" if arg is None else json.dumps(arg, ensure_ascii=False)
    return f"{load_xhs_page_scripts()}\n// SOCAI_XHS_CALL: {name}\nreturn SocaiXhsPageScripts.{name}({args});"


def _normalize_keyword(value: str) -> str:
    return re.sub(r"\s+", " ", str(value or "").strip()).lower()


def _search_transition_ok(state: dict, query: str) -> bool:
    if str(state.get("page_state") or "") != "search_results":
        return False

    keyword = _normalize_keyword(query)
    visible_keyword = _normalize_keyword(str(state.get("input_keyword") or ""))
    url_keyword = _normalize_keyword(str(state.get("url_keyword") or ""))
    if keyword and visible_keyword and visible_keyword != keyword:
        return False
    if keyword and not visible_keyword and url_keyword and url_keyword != keyword:
        return False

    if int(state.get("card_count") or 0) > 0:
        return True
    if state.get("loading"):
        return False
    return bool(state.get("has_no_results"))


JS_DISPATCH_ESCAPE = (
    "document.dispatchEvent(new KeyboardEvent('keydown', "
    "{key: 'Escape', code: 'Escape', keyCode: 27, which: 27, bubbles: true}))"
)


class XhsRuntime:
    """Site-aware operations for Xiaohongshu."""

    def __init__(self, page: PageSession):
        self.page = page
        # Track the last note_id we extracted so we can warn the agent if it
        # extracts the same note twice without closing the modal in between —
        # a common failure mode where a stale overlay masks the new card click.
        self._last_extracted_note_id: str = ""

    async def current_url(self) -> str:
        info = await self.page.page_info()
        return str(info.get("url") or "")

    async def ensure_xhs(self, *, navigate_if_needed: bool = False) -> None:
        url = await self.current_url()
        if "xiaohongshu.com" in url:
            return
        if navigate_if_needed:
            await self.page.navigate(XHS_HOME_URL)
            return
        raise RuntimeError(f"Current page is not Xiaohongshu: {url or 'unknown'}")

    async def run_page_script(self, name: str, *, expected_type: type | tuple[type, ...], arg: Any = None) -> Any:
        value = await self.page.evaluate(xhs_page_script_call(name, arg))
        if not isinstance(value, expected_type):
            expected = (
                " or ".join(item.__name__ for item in expected_type)
                if isinstance(expected_type, tuple)
                else expected_type.__name__
            )
            raise RuntimeError(f"XHS page script {name} returned {type(value).__name__}, expected {expected}")
        return value

    async def search_notes(self, query: str, *, wait_seconds: float = 2.0) -> dict:
        keyword = str(query or "").strip()
        if not keyword:
            raise ValueError("query is required")

        await self.ensure_xhs(navigate_if_needed=True)
        submit = await self.submit_search(keyword, wait_seconds=wait_seconds)
        ok = bool(submit.get("ok"))
        cards = await self.extract_search_cards() if ok else []
        return {
            "ok": ok,
            "query": keyword,
            "submit": submit,
            "url": await self.current_url(),
            "count": len(cards),
            "cards": [card.to_dict() for card in cards],
            "reason": "" if ok else str(submit.get("error") or "search_submit_failed"),
        }

    async def submit_search(self, query: str, *, wait_seconds: float = 2.0) -> dict:
        loc = await self.run_page_script("searchInput", expected_type=dict)
        if not loc.get("ok"):
            return {"ok": False, "strategy": "search_input_unavailable", "error": loc.get("error", "")}

        input_pos = loc.get("input") or {}
        await self.page.click(float(input_pos.get("x", 0)), float(input_pos.get("y", 0)))
        await asyncio.sleep(0.2)

        await self.page.type_text(query)
        await asyncio.sleep(0.2)
        input_state = await self.run_page_script("searchState", expected_type=dict)
        if _normalize_keyword(str(input_state.get("input_keyword") or "")) != _normalize_keyword(query):
            return {
                "ok": False,
                "strategy": "type_search_input_failed",
                "state": input_state,
                "error": "Search input did not accept the requested keyword",
            }

        await asyncio.sleep(0.1)
        await self.page.press_key("Enter")
        state = await self.wait_for_search_transition(query, timeout_s=max(0.2, min(float(wait_seconds), 6.0)))
        if _search_transition_ok(state, query):
            return {
                "ok": True,
                "strategy": "click_input_set_value_enter",
                "state": state,
                "url": await self.current_url(),
            }

        submit_pos = loc.get("submit") or {}
        if submit_pos.get("x") and submit_pos.get("y"):
            await self.page.click(float(submit_pos["x"]), float(submit_pos["y"]))
            state = await self.wait_for_search_transition(query, timeout_s=max(0.2, min(float(wait_seconds), 6.0)))
            if _search_transition_ok(state, query):
                return {
                    "ok": True,
                    "strategy": "click_search_button",
                    "state": state,
                    "url": await self.current_url(),
                }

        return {
            "ok": False,
            "strategy": "manual_submit_failed",
            "state": state,
            "url": await self.current_url(),
            "error": "Search did not transition to a valid Xiaohongshu result page",
        }

    async def wait_for_search_transition(
        self,
        query: str,
        *,
        timeout_s: float = 2.0,
        poll_s: float = 0.15,
    ) -> dict:
        deadline = asyncio.get_running_loop().time() + max(0.2, float(timeout_s))
        latest: dict = {}
        while asyncio.get_running_loop().time() < deadline:
            latest = await self.run_page_script("searchState", expected_type=dict)
            if _search_transition_ok(latest, query):
                return latest
            await asyncio.sleep(max(0.05, float(poll_s)))
        return latest or await self.run_page_script("searchState", expected_type=dict)

    async def extract_search_cards(self) -> list[XhsNoteCard]:
        await self.ensure_xhs(navigate_if_needed=False)
        raw = await self.run_page_script("searchCards", expected_type=list)
        cards: list[XhsNoteCard] = []
        for index, item in enumerate(raw):
            if not isinstance(item, dict):
                continue
            cards.append(
                XhsNoteCard(
                    note_id=str(item.get("note_id") or ""),
                    title=str(item.get("title") or ""),
                    author=str(item.get("author") or ""),
                    likes=str(item.get("likes") or ""),
                    link=str(item.get("link") or ""),
                    cover_url=str(item.get("cover_url") or ""),
                    type=str(item.get("type") or ""),
                    position=int(item.get("position", index) or index),
                    xsec_token=str(item.get("xsec_token") or ""),
                )
            )
        return cards

    async def open_note(
        self,
        *,
        note_id: str = "",
        index: int | None = None,
        wait_seconds: float = 4.0,
    ) -> dict:
        """Open a note by simulating a human click on the card.

        Direct ``Page.navigate`` to ``/explore/<id>`` triggers Xiaohongshu's
        anti-bot path (captcha or "scan in app" prompt). We instead locate the
        card in the current grid, fetch its center coordinates from JS, and
        dispatch a real CDP mouse click — which opens the note as a modal.
        """

        cards = await self.extract_search_cards()
        selected: XhsNoteCard | None = None
        if note_id:
            selected = next((card for card in cards if card.note_id == note_id), None)
        if selected is None and index is not None and 0 <= int(index) < len(cards):
            selected = cards[int(index)]
        if selected is None:
            raise RuntimeError("Could not resolve note target from current search cards.")

        click_arg: dict[str, Any] = {}
        if selected.note_id:
            click_arg["note_id"] = selected.note_id
        if index is not None:
            click_arg["index"] = int(index)
        elif selected.position is not None:
            click_arg["index"] = int(selected.position)

        target = await self.run_page_script("clickCard", expected_type=dict, arg=click_arg)
        if not target.get("ok"):
            raise RuntimeError(
                f"Could not locate card to click for note {selected.note_id or selected.position}: "
                f"{target.get('error')}"
            )

        # Reset the stale-overlay tracker for the new attempt.
        self._last_extracted_note_id = ""

        per_attempt = max(0.2, float(wait_seconds) / 2)
        click_target_kind = str(target.get("target") or "cover")

        await self.page.click(float(target["x"]), float(target["y"]))
        opened = await self._wait_for_note_open(timeout_s=per_attempt)
        if opened.get("on_detail_route") or opened.get("has_modal"):
            return {
                "ok": True,
                "target": selected.to_dict(),
                "url": await self.current_url(),
                "state": opened,
                "strategy": f"{click_target_kind}_click",
            }

        # Fallback: re-issue clickCard so the click coords reflect any layout
        # shift (scrolling, ad insertion), then click the card body itself.
        # Mirrors flowlens' two-stage click_card_or_card.click() path.
        retry = await self.run_page_script("clickCard", expected_type=dict, arg=click_arg)
        if retry.get("ok"):
            await self.page.click(float(retry["x"]), float(retry["y"]))
            opened = await self._wait_for_note_open(timeout_s=per_attempt)
            if opened.get("on_detail_route") or opened.get("has_modal"):
                return {
                    "ok": True,
                    "target": selected.to_dict(),
                    "url": await self.current_url(),
                    "state": opened,
                    "strategy": f"retry_{retry.get('target') or 'card'}_click",
                }

        return {
            "ok": False,
            "target": selected.to_dict(),
            "url": await self.current_url(),
            "state": opened,
            "strategy": "card_click_failed",
            "error": "Note overlay did not open after card-click attempts; site may be throttling or layout changed",
        }

    async def close_note(self, *, wait_seconds: float = 1.5) -> dict:
        """Close an open note modal with multiple human-like strategies.

        Order:
        1. CDP ``Input.dispatchKeyEvent`` Escape — the cleanest path when XHS
           listens on ``window``.
        2. JS-level ``document.dispatchEvent(KeyboardEvent('keydown'))`` —
           some XHS React handlers attach directly to ``document`` and miss
           the CDP path.
        3. Click the close-button at coordinates returned by the JS extractor.
        """

        before = await self.run_page_script("noteOpen", expected_type=dict)
        if not (before.get("on_detail_route") or before.get("has_modal")):
            self._last_extracted_note_id = ""
            return {"ok": True, "strategy": "already_closed", "state": before}

        per_attempt = max(0.2, float(wait_seconds))

        # Strategy 1: real keyboard event via CDP.
        await self.page.press_key("Escape")
        state = await self._wait_for_note_closed(timeout_s=per_attempt)
        if not (state.get("on_detail_route") or state.get("has_modal")):
            self._last_extracted_note_id = ""
            return {"ok": True, "strategy": "escape", "state": state, "url": await self.current_url()}

        # Strategy 2: synthetic KeyboardEvent at the document level.
        try:
            await self.page.evaluate(JS_DISPATCH_ESCAPE)
        except Exception:  # noqa: BLE001 - best-effort fallback
            pass
        state = await self._wait_for_note_closed(timeout_s=per_attempt)
        if not (state.get("on_detail_route") or state.get("has_modal")):
            self._last_extracted_note_id = ""
            return {"ok": True, "strategy": "escape_dispatch", "state": state, "url": await self.current_url()}

        # Strategy 3: locate and click the close button.
        close_btn = await self.run_page_script("closeNote", expected_type=dict)
        if close_btn.get("ok"):
            await self.page.click(float(close_btn["x"]), float(close_btn["y"]))
            state = await self._wait_for_note_closed(timeout_s=per_attempt)
            if not (state.get("on_detail_route") or state.get("has_modal")):
                self._last_extracted_note_id = ""
                return {
                    "ok": True,
                    "strategy": "close_button",
                    "selector": close_btn.get("selector", ""),
                    "state": state,
                    "url": await self.current_url(),
                }

        return {
            "ok": False,
            "strategy": "close_failed",
            "state": state,
            "url": await self.current_url(),
            "error": "Note modal did not close after Escape, JS-dispatch Escape, and close-button attempts",
        }

    async def _wait_for_note_open(self, *, timeout_s: float, poll_s: float = 0.15) -> dict:
        deadline = asyncio.get_running_loop().time() + max(0.2, float(timeout_s))
        latest: dict = {}
        while asyncio.get_running_loop().time() < deadline:
            latest = await self.run_page_script("noteOpen", expected_type=dict)
            if latest.get("on_detail_route") or latest.get("has_modal"):
                return latest
            await asyncio.sleep(max(0.05, float(poll_s)))
        return latest or await self.run_page_script("noteOpen", expected_type=dict)

    async def _wait_for_note_closed(self, *, timeout_s: float, poll_s: float = 0.15) -> dict:
        deadline = asyncio.get_running_loop().time() + max(0.2, float(timeout_s))
        latest: dict = {}
        while asyncio.get_running_loop().time() < deadline:
            latest = await self.run_page_script("noteOpen", expected_type=dict)
            if not (latest.get("on_detail_route") or latest.get("has_modal")):
                return latest
            await asyncio.sleep(max(0.05, float(poll_s)))
        return latest or await self.run_page_script("noteOpen", expected_type=dict)

    async def extract_note(self, *, wait_seconds: float = 8.0) -> XhsNote:
        """Extract the currently open note.

        Uses ``noteWithWait`` so the JS side polls until content has hydrated
        (or the shell has settled long enough to trust an empty body), giving
        us one round-trip instead of polling from Python.
        """

        await self.ensure_xhs(navigate_if_needed=False)
        raw = await self.run_page_script(
            "noteWithWait",
            expected_type=dict,
            arg={"timeout_ms": int(max(0.5, float(wait_seconds)) * 1000)},
        )
        wait_meta = {
            "ready": bool(raw.get("ready")),
            "reason": str(raw.get("reason") or ""),
            "waited_ms": int(raw.get("waited_ms") or 0),
            "attempts": int(raw.get("attempts") or 0),
        }
        body = raw.get("note") or {}
        if not isinstance(body, dict):
            body = {}
        hashtags = body.get("hashtags") if isinstance(body.get("hashtags"), list) else []
        note_id = str(body.get("note_id") or "")
        stale = ""
        if note_id and self._last_extracted_note_id and note_id == self._last_extracted_note_id:
            stale = (
                f"This note (note_id={note_id}) was already extracted in the previous read. "
                "The note modal may not have closed before opening the next card — "
                "call xhs_close_note to verify the modal is gone, then re-open the target card."
            )
        if note_id:
            self._last_extracted_note_id = note_id
        note = XhsNote(
            note_id=note_id,
            url=str(body.get("url") or await self.current_url()),
            type=str(body.get("type") or ""),
            title=str(body.get("title") or ""),
            author=str(body.get("author") or ""),
            content=str(body.get("content") or ""),
            content_source=str(body.get("content_source") or ""),
            hashtags=[str(tag) for tag in hashtags if str(tag).strip()],
            likes=str(body.get("likes") or ""),
            favorites=str(body.get("favorites") or ""),
            comments_count=str(body.get("comments_count") or ""),
        )
        note.stale_warning = stale
        note.wait_meta = wait_meta
        return note

    async def extract_comments(
        self,
        *,
        prefer_hot: bool = True,
        max_comments: int = 20,
    ) -> list[dict]:
        await self.ensure_xhs(navigate_if_needed=False)
        arg: dict[str, Any] = {"prefer_hot": bool(prefer_hot)}
        if max_comments:
            arg["max_comments"] = int(max_comments)
        raw = await self.run_page_script("comments", expected_type=list, arg=arg)
        return [item for item in raw if isinstance(item, dict)]

    async def scroll_in_note(self, *, pixels: int = 400) -> dict:
        await self.ensure_xhs(navigate_if_needed=False)
        return await self.run_page_script("scrollInNote", expected_type=dict, arg={"pixels": int(pixels)})

    async def list_search_tabs(self) -> list[dict]:
        await self.ensure_xhs(navigate_if_needed=False)
        raw = await self.run_page_script("searchTabs", expected_type=list)
        return [item for item in raw if isinstance(item, dict)]

    async def click_search_tab(self, label: str, *, wait_seconds: float = 1.5) -> dict:
        await self.ensure_xhs(navigate_if_needed=False)
        loc = await self.run_page_script("clickSearchTab", expected_type=dict, arg=str(label))
        if not loc.get("ok"):
            return {"ok": False, "label": label, "error": loc.get("error", "unknown")}
        if loc.get("was_active"):
            return {"ok": True, "label": label, "skipped": True, "reason": "already_active"}
        await self.page.click(float(loc["x"]), float(loc["y"]))
        if wait_seconds > 0:
            await asyncio.sleep(min(float(wait_seconds), 4.0))
        tabs = await self.list_search_tabs()
        return {
            "ok": True,
            "label": label,
            "active_filter": next((t["label"] for t in tabs if t.get("active")), ""),
            "tabs": tabs,
        }

    async def read_note(
        self,
        *,
        note_id: str = "",
        index: int | None = None,
        with_comments: bool = True,
        max_comments: int = 12,
    ) -> dict:
        if note_id or index is not None:
            opened = await self.open_note(note_id=note_id, index=index)
            if not opened.get("ok"):
                return {"ok": False, "open": opened, "error": opened.get("error", "open_failed")}
        note = await self.extract_note()
        payload: dict[str, Any] = {"ok": True, "entity": note.to_dict()}
        if with_comments:
            try:
                payload["comments"] = await self.extract_comments(max_comments=max_comments)
            except Exception as exc:  # noqa: BLE001 - comments are best-effort
                payload["comments"] = []
                payload["comments_error"] = str(exc)
        return payload


def extract_note_id_from_url(url: str) -> str:
    match = re.search(r"/(?:explore|search_result|discovery)/([^/?#]+)", str(url or ""))
    return match.group(1) if match else ""
