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
EXTRACTOR_JS = Path(__file__).with_name("extractors.js")
EXTRACTOR_FUNCTIONS = {
    "note",
    "searchCards",
    "searchInput",
    "searchState",
}


@lru_cache(maxsize=1)
def load_extractors() -> str:
    return EXTRACTOR_JS.read_text(encoding="utf-8")


def extractor_call(name: str, arg: Any = None) -> str:
    if name not in EXTRACTOR_FUNCTIONS:
        raise ValueError(f"Unknown XHS extractor: {name}")
    args = "" if arg is None else json.dumps(arg, ensure_ascii=False)
    return f"{load_extractors()}\n// SOCAI_XHS_CALL: {name}\nreturn SocaiXhsExtractors.{name}({args});"


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


class XhsRuntime:
    """Site-aware operations for Xiaohongshu."""

    def __init__(self, page: PageSession):
        self.page = page

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

    async def run_extractor(self, name: str, *, expected_type: type | tuple[type, ...], arg: Any = None) -> Any:
        value = await self.page.evaluate(extractor_call(name, arg))
        if not isinstance(value, expected_type):
            expected = (
                " or ".join(item.__name__ for item in expected_type)
                if isinstance(expected_type, tuple)
                else expected_type.__name__
            )
            raise RuntimeError(f"XHS extractor {name} returned {type(value).__name__}, expected {expected}")
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
        loc = await self.run_extractor("searchInput", expected_type=dict)
        if not loc.get("ok"):
            return {"ok": False, "strategy": "search_input_unavailable", "error": loc.get("error", "")}

        input_pos = loc.get("input") or {}
        await self.page.click(float(input_pos.get("x", 0)), float(input_pos.get("y", 0)))
        await asyncio.sleep(0.2)

        await self.page.type_text(query)
        await asyncio.sleep(0.2)
        input_state = await self.run_extractor("searchState", expected_type=dict)
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
            latest = await self.run_extractor("searchState", expected_type=dict)
            if _search_transition_ok(latest, query):
                return latest
            await asyncio.sleep(max(0.05, float(poll_s)))
        return latest or await self.run_extractor("searchState", expected_type=dict)

    async def extract_search_cards(self) -> list[XhsNoteCard]:
        await self.ensure_xhs(navigate_if_needed=False)
        raw = await self.run_extractor("searchCards", expected_type=list)
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

    async def open_note(self, *, note_id: str = "", index: int | None = None) -> dict:
        cards = await self.extract_search_cards()
        selected: XhsNoteCard | None = None
        if note_id:
            selected = next((card for card in cards if card.note_id == note_id), None)
        if selected is None and index is not None and 0 <= int(index) < len(cards):
            selected = cards[int(index)]
        if selected is None:
            raise RuntimeError("Could not resolve note target from current search cards.")
        if not selected.link:
            raise RuntimeError(f"Selected note has no openable link: {selected.to_dict()}")
        await self.page.navigate(selected.link)
        return {"ok": True, "target": selected.to_dict(), "url": await self.current_url()}

    async def extract_note(self) -> XhsNote:
        await self.ensure_xhs(navigate_if_needed=False)
        raw = await self.run_extractor("note", expected_type=dict)
        hashtags = raw.get("hashtags") if isinstance(raw.get("hashtags"), list) else []
        return XhsNote(
            note_id=str(raw.get("note_id") or ""),
            url=str(raw.get("url") or await self.current_url()),
            title=str(raw.get("title") or ""),
            author=str(raw.get("author") or ""),
            content=str(raw.get("content") or ""),
            hashtags=[str(tag) for tag in hashtags if str(tag).strip()],
            likes=str(raw.get("likes") or ""),
            favorites=str(raw.get("favorites") or ""),
            comments_count=str(raw.get("comments_count") or ""),
            raw_text_excerpt=str(raw.get("raw_text_excerpt") or ""),
        )

    async def read_note(self, *, note_id: str = "", index: int | None = None) -> dict:
        if note_id or index is not None:
            await self.open_note(note_id=note_id, index=index)
        note = await self.extract_note()
        return {"ok": True, "entity": note.to_dict()}


def extract_note_id_from_url(url: str) -> str:
    match = re.search(r"/(?:explore|search_result|discovery)/([^/?#]+)", str(url or ""))
    return match.group(1) if match else ""
