"""Agent tools for the Xiaohongshu site runtime.

Each tool here is a thin LLM-shape adapter over ``XhsRuntime``. The shared
``_emit`` helper handles three responsibilities common to every persistent
tool: tagging the payload with ``{site, action}``, writing it as a JSON
artifact under ``run_dir/site_results/``, and returning a truncated reply
for the model (full data lives on disk, agent context stays small).
"""

from __future__ import annotations

import json
import time

from socai.agent.tool import Tool, ToolContext
from socai.media.timing import TimingRecord

from .entities import XhsNoteCard
from .history import XhsHistoryStore
from .runtime import XhsRuntime


def _json(payload: dict) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2)


_LEVEL_ORDER = {"card": 0, "lite": 1, "deep": 2}
_SCAN_PROFILES = {
    "quick": {"deep": 0, "lite": 3, "deep_comments": 0, "lite_comments": 4, "media": False},
    "standard": {"deep": 2, "lite": 3, "deep_comments": 10, "lite_comments": 4, "media": False},
    "deep": {"deep": 3, "lite": 3, "deep_comments": 12, "lite_comments": 6, "media": True},
}


def _level_value(level: str) -> int:
    return _LEVEL_ORDER.get(str(level or "").strip().lower(), -1)


def _artifact_from_reply(reply: str) -> str:
    try:
        payload = json.loads(reply)
    except Exception:
        return ""
    return str(payload.get("artifact") or "") if isinstance(payload, dict) else ""


def _record_processed_note(
    ctx: ToolContext,
    entity: dict,
    artifact: str,
    *,
    level: str,
    include_media: bool,
) -> None:
    note_id = str(entity.get("note_id") or "").strip()
    if not note_id:
        return
    prior = ctx.processed_notes.get(note_id) or {}
    keep_prior = prior and (
        _level_value(str(prior.get("level") or "")) > _level_value(level)
        or (
            _level_value(str(prior.get("level") or "")) == _level_value(level)
            and bool(prior.get("include_media"))
            and not include_media
        )
    )
    if keep_prior:
        prior["artifact"] = artifact or prior.get("artifact", "")
        ctx.processed_notes[note_id] = prior
        return
    ctx.processed_notes[note_id] = {
        "title": str(entity.get("title") or prior.get("title") or ""),
        "artifact": artifact,
        "level": level,
        "include_media": bool(include_media),
    }


def _dedup_short_circuit(
    ctx: ToolContext,
    *,
    note_id: str,
    requested_level: str,
    requested_include_media: bool,
    force: bool,
) -> str | None:
    if force or not note_id:
        return None
    info = ctx.processed_notes.get(str(note_id).strip())
    if not isinstance(info, dict):
        return None
    prior_level = str(info.get("level") or "")
    prior_include_media = bool(info.get("include_media", False))
    if _level_value(prior_level) < _level_value(requested_level):
        return None
    if requested_include_media and not prior_include_media:
        return None
    processed_ids = [str(nid) for nid in ctx.processed_notes if nid]
    sampled_ids = [str(nid) for nid in ctx.topic_scan_note_ids if nid]
    remaining_ids = [nid for nid in sampled_ids if nid not in processed_ids]
    return _json(
        {
            "site": "xiaohongshu",
            "ok": True,
            "skipped": True,
            "reason": "already_analyzed",
            "note_id": note_id,
            "title": info.get("title", ""),
            "prior_level": prior_level,
            "prior_include_media": prior_include_media,
            "requested_level": requested_level,
            "requested_include_media": requested_include_media,
            "prior_artifact": info.get("artifact", ""),
            "already_processed_note_ids": processed_ids,
            "remaining_sampled_note_ids": remaining_ids,
            "next_action": (
                "Read another sampled note if remaining_sampled_note_ids is non-empty; "
                "otherwise stop reading and summarize from saved artifacts."
            ),
        }
    )


def _annotate_cards(ctx: ToolContext, history: XhsHistoryStore, cards: list[dict]) -> list[dict]:
    annotated: list[dict] = []
    for card in cards:
        out = history.annotate_card(card)
        note_id = str(out.get("note_id") or "").strip()
        if note_id and note_id in ctx.processed_notes:
            prior = ctx.processed_notes[note_id]
            out["already_analyzed"] = True
            out["prior_artifact"] = prior.get("artifact", "")
            out["prior_level"] = prior.get("level", "")
            out["prior_include_media"] = bool(prior.get("include_media", False))
        annotated.append(out)
    return annotated


class XhsToolBase(Tool):
    SITE = "xiaohongshu"

    def __init__(self, runtime: XhsRuntime):
        self.runtime = runtime
        self.history = XhsHistoryStore()

    @property
    def defer_until_site(self) -> str:
        return self.SITE

    async def _emit(
        self,
        ctx: ToolContext,
        label: str,
        payload: dict,
        *,
        preview: dict | None = None,
    ) -> str:
        """Persist ``payload`` as a JSON artifact and return a reply for the LLM.

        Pass ``preview`` to send a slimmer view to the model (e.g. truncate
        long lists). When ``preview`` is None, the full payload is returned —
        appropriate for small results like a single extracted entity.
        """

        tagged = {"site": self.SITE, "action": self.name, **payload}
        artifact = ctx.write_json_artifact(
            label,
            tagged,
            subdir="site_results",
            source_tool=self.name,
            artifact_kind="site_result",
            summary=str(tagged.get("summary") or self.name),
            metadata={"site": self.SITE},
        )
        body = preview if preview is not None else {
            k: v for k, v in tagged.items() if k not in {"site", "action"}
        }
        return _json({**body, "artifact": artifact})


# ── persistent tools (write site_results artifact) ───────────────────────────


class XhsSearchNotesTool(XhsToolBase):
    name = "xhs_search_notes"
    description = (
        "Search Xiaohongshu using the desktop search_result route and return visible/loaded note cards. "
        "Cards include stable note_id and tokenized link when available."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "query": {"type": "string"},
            },
            "required": ["query"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        query = str(params.get("query") or "")
        payload = await self.runtime.search_notes(query)
        payload["cards"] = _annotate_cards(ctx, self.history, payload.get("cards") or [])
        return await self._emit(
            ctx,
            label=f"xhs_search_{query or 'query'}",
            payload=payload,
            preview={"ok": payload["ok"], "count": payload["count"], "cards": payload["cards"][:8]},
        )


class XhsExtractSearchCardsTool(XhsToolBase):
    name = "xhs_extract_search_cards"
    description = "Extract Xiaohongshu note cards from the current search/profile grid without navigating."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        cards = [card.to_dict() for card in await self.runtime.extract_search_cards()]
        cards = _annotate_cards(ctx, self.history, cards)
        payload = {"ok": True, "count": len(cards), "cards": cards}
        return await self._emit(
            ctx,
            label="xhs_search_cards",
            payload=payload,
            preview={"ok": True, "count": len(cards), "cards": cards[:8]},
        )


class XhsReadNoteTool(XhsToolBase):
    name = "xhs_read_note"
    description = (
        "Open a Xiaohongshu note (by note_id or index from current cards) and extract the note "
        "body. level=card is metadata only, lite adds comments, deep can add optional OCR/vision/video media."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "note_id": {"type": "string"},
                "index": {"type": "integer"},
                "level": {"type": "string", "enum": ["card", "lite", "deep"], "default": "lite"},
                "include_media": {"type": "boolean", "default": False},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        index = params.get("index")
        level = str(params.get("level") or "lite").lower()
        include_media = bool(params.get("include_media", False))
        note_id = str(params.get("note_id") or "")
        short_circuit = _dedup_short_circuit(
            ctx,
            note_id=note_id,
            requested_level=level,
            requested_include_media=include_media,
            force=False,
        )
        if short_circuit:
            return short_circuit
        with_comments = level != "card"
        max_comments = 0 if level == "card" else (12 if level == "deep" else 6)
        payload = await self.runtime.read_note(
            note_id=note_id,
            index=int(index) if index is not None else None,
            level=level,
            with_comments=with_comments,
            max_comments=max_comments,
            include_media=include_media,
        )
        entity = payload.get("entity") or {}
        comments = payload.get("comments") or []
        if isinstance(entity, dict):
            entity["top_comments"] = comments[:5]
        label = f"xhs_note_{entity.get('note_id') or 'note'}"
        reply = await self._emit(
            ctx,
            label=label,
            payload=payload,
            preview={"ok": payload.get("ok", False), "entity": entity, "comments": comments[:8]},
        )
        artifact = _artifact_from_reply(reply)
        if payload.get("ok") and isinstance(entity, dict):
            _record_processed_note(ctx, entity, artifact, level=level, include_media=include_media)
            self.history.record_note(
                entity,
                artifact=artifact,
                run_dir=str(ctx.run_dir),
                level=level,
                include_media=include_media,
                source_tool=self.name,
            )
        return reply


class XhsExtractNoteTool(XhsToolBase):
    name = "xhs_extract_note"
    description = "Extract a Xiaohongshu note entity from the currently open note page (no comments)."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "level": {"type": "string", "enum": ["card", "lite", "deep"], "default": "lite"},
                "include_media": {"type": "boolean", "default": False},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        level = str(params.get("level") or "lite")
        include_media = bool(params.get("include_media", False))
        note = await self.runtime.extract_note(
            level=level,
            include_media=include_media,
        )
        payload = {"ok": True, "entity": note.to_dict()}
        reply = await self._emit(ctx, label=f"xhs_note_{note.note_id or 'current'}", payload=payload)
        artifact = _artifact_from_reply(reply)
        _record_processed_note(ctx, note.to_dict(), artifact, level=level, include_media=include_media)
        self.history.record_note(
            note.to_dict(),
            artifact=artifact,
            run_dir=str(ctx.run_dir),
            level=level,
            include_media=include_media,
            source_tool=self.name,
        )
        return reply


class XhsExtractCommentsTool(XhsToolBase):
    name = "xhs_extract_comments"
    description = (
        "Extract comments from the currently open Xiaohongshu note. Top-level comments include "
        "nested sub_comments. Sorted by heat (likes + replies + pinned bonus) by default."
    )

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        comments = await self.runtime.extract_comments(prefer_hot=True, max_comments=12)
        payload = {"ok": True, "count": len(comments), "comments": comments}
        return await self._emit(
            ctx,
            label="xhs_comments",
            payload=payload,
            preview={"ok": True, "count": len(comments), "comments": comments[:8]},
        )


class XhsExtractProfileTool(XhsToolBase):
    name = "xhs_extract_profile"
    description = "Extract the current Xiaohongshu creator profile and visible note cards."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        profile = await self.runtime.extract_profile()
        payload = {"ok": True, "profile": profile.to_dict()}
        note_cards = payload["profile"].get("note_cards") or []
        payload["profile"]["note_cards"] = _annotate_cards(ctx, self.history, note_cards)
        return await self._emit(
            ctx,
            label=f"xhs_profile_{profile.xhs_id or profile.display_name or 'current'}",
            payload=payload,
            preview={
                "ok": True,
                "profile": {
                    **{k: v for k, v in payload["profile"].items() if k != "note_cards"},
                    "note_cards": payload["profile"]["note_cards"][:10],
                },
            },
        )


class XhsTopicScanTool(XhsToolBase):
    name = "xhs_topic_scan"
    description = (
        "Xiaohongshu topic research macro: search -> optional tab switch -> sample visible cards "
        "in page order -> read deep/lite notes -> write one compact artifact. Prefer this for XHS topic research. "
        "Do not repeat the same scan at a deeper depth unless the previous scan was clearly insufficient."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "tab_label": {"type": "string", "enum": ["全部", "图文", "视频", "用户"]},
                "depth": {
                    "type": "string",
                    "enum": ["quick", "standard", "deep"],
                    "default": "standard",
                    "description": "Use deep only when broader note/comment evidence is required.",
                },
            },
            "required": ["query"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        query = str(params.get("query") or "").strip()
        if not query:
            raise ValueError("query is required")
        depth = str(params.get("depth") or "standard").strip().lower()
        profile = _SCAN_PROFILES.get(depth) or _SCAN_PROFILES["standard"]
        max_deep = int(profile["deep"])
        max_lite = int(profile["lite"])
        total_limit = max_deep + max_lite
        include_media = bool(profile["media"])

        scan_timing = TimingRecord()
        # Snapshot media-processor counters so per-scan timing only reflects
        # work done by this invocation, not earlier scans in the same run.
        media_baseline = (
            dict(self.runtime.media.timing.totals) if getattr(self.runtime, "media", None) else {}
        )
        media_count_baseline = (
            dict(self.runtime.media.timing.counts) if getattr(self.runtime, "media", None) else {}
        )
        scan_t0 = time.perf_counter()

        with scan_timing.measure("xhs_search_notes"):
            search = await self.runtime.search_notes(query)
        tab_label = str(params.get("tab_label") or "").strip()
        tab_result: dict = {}
        if tab_label:
            with scan_timing.measure("xhs_click_search_tab"):
                tab_result = await self.runtime.click_search_tab(tab_label)
            with scan_timing.measure("xhs_extract_search_cards"):
                cards = [card.to_dict() for card in await self.runtime.extract_search_cards()]
        else:
            cards = search.get("cards") or []

        card_entities = []
        for card_dict in cards:
            try:
                card_entities.append(self._card_from_dict(card_dict))
            except Exception:
                continue
        selected = self._select_cards(card_entities, total_limit=total_limit)
        sampled_ids = [card.note_id for card in selected if card.note_id]
        ctx.topic_scan_note_ids = list(dict.fromkeys([*ctx.topic_scan_note_ids, *sampled_ids]))

        notes: list[dict] = []
        for index, card in enumerate(selected):
            level = "deep" if index < max_deep else "lite"
            comment_count = int(profile["deep_comments"]) if level == "deep" else int(profile["lite_comments"])
            short_circuit = _dedup_short_circuit(
                ctx,
                note_id=card.note_id,
                requested_level=level,
                requested_include_media=include_media and level == "deep",
                force=False,
            )
            if short_circuit:
                notes.append({
                    "scan_level": level,
                    "source_position": card.position,
                    "skipped": json.loads(short_circuit),
                    "entity": card.to_dict(),
                })
                continue
            try:
                read_t0 = time.perf_counter()
                payload = await self.runtime.read_note(
                    note_id=card.note_id,
                    index=None if card.note_id else card.position,
                    level=level,
                    with_comments=True,
                    max_comments=comment_count,
                    include_media=include_media and level == "deep",
                )
                scan_timing.record(f"read_note_{level}", time.perf_counter() - read_t0)
                entity = payload.get("entity") or {}
                if isinstance(entity, dict):
                    entity["top_comments"] = (payload.get("comments") or [])[:5]
                    try:
                        screenshot_path = ctx.next_screenshot_path(f"xhs_topic_{index + 1}_{level}")
                        with scan_timing.measure("note_screenshot"):
                            await self.runtime.page.screenshot(screenshot_path, max_dim=1600)
                        screenshot_rel = ctx.register_artifact(
                            screenshot_path,
                            label=f"xhs_topic_{index + 1}_{level}",
                            artifact_kind="image",
                            summary=f"XHS topic scan screenshot: {entity.get('title') or card.title}",
                            metadata={"site": self.SITE, "category": "screenshot"},
                            source_tool=self.name,
                        )
                        entity["screenshot"] = screenshot_rel
                    except Exception:
                        pass
                    _record_processed_note(
                        ctx,
                        entity,
                        f"topic_scan:{query}#{index + 1}",
                        level=level,
                        include_media=include_media and level == "deep",
                    )
                notes.append({
                    "scan_level": level,
                    "source_position": card.position,
                    "ok": bool(payload.get("ok")),
                    "entity": entity,
                    "comments": (payload.get("comments") or [])[:comment_count],
                    "error": payload.get("error", ""),
                })
            except Exception as exc:  # noqa: BLE001 - continue sampling
                notes.append({
                    "scan_level": level,
                    "source_position": card.position,
                    "ok": False,
                    "entity": card.to_dict(),
                    "error": str(exc),
                })
            finally:
                try:
                    await self.runtime.close_note()
                except Exception:
                    pass

        selected_cards = [card.to_dict() for card in selected]
        scan_total_s = round(time.perf_counter() - scan_t0, 3)
        media_timing_delta = self._media_timing_delta(media_baseline, media_count_baseline)
        timing_summary = {
            "scan_total_s": scan_total_s,
            "scan_phases": scan_timing.summary(),
            "media": media_timing_delta,
        }
        payload = {
            "ok": bool(search.get("ok")),
            "query": query,
            "tab": tab_result,
            "search": {**search, "cards": _annotate_cards(ctx, self.history, search.get("cards") or [])},
            "selected_cards": _annotate_cards(ctx, self.history, selected_cards),
            "notes": notes,
            "sampling": {
                "max_deep_notes": max_deep,
                "max_lite_notes": max_lite,
                "depth": depth,
                "include_media": include_media,
            },
            "timing": timing_summary,
        }
        reply = await self._emit(
            ctx,
            label=f"xhs_topic_scan_{query}",
            payload=payload,
            preview={
                "ok": payload["ok"],
                "query": query,
                "selected_cards": payload["selected_cards"][:8],
                "notes": [
                    {
                        "scan_level": note.get("scan_level"),
                        "ok": note.get("ok", True),
                        "entity": note.get("entity"),
                        "error": note.get("error", ""),
                    }
                    for note in notes[:8]
                ],
                "timing": timing_summary,
            },
        )
        artifact = _artifact_from_reply(reply)
        for note in notes:
            entity = note.get("entity")
            if isinstance(entity, dict) and entity.get("note_id"):
                self.history.record_note(
                    entity,
                    artifact=artifact,
                    run_dir=str(ctx.run_dir),
                    level=str(note.get("scan_level") or ""),
                    include_media=include_media and note.get("scan_level") == "deep",
                    source_tool=self.name,
                )
        return reply

    def _media_timing_delta(
        self,
        baseline_totals: dict,
        baseline_counts: dict,
    ) -> dict[str, dict[str, float]]:
        media = getattr(self.runtime, "media", None)
        if media is None:
            return {}
        result: dict[str, dict[str, float]] = {}
        for op, total in media.timing.totals.items():
            base_total = float(baseline_totals.get(op, 0.0))
            base_count = int(baseline_counts.get(op, 0))
            delta_total = max(0.0, float(total) - base_total)
            delta_count = max(0, int(media.timing.counts.get(op, 0)) - base_count)
            if delta_count == 0 and delta_total <= 0:
                continue
            result[op] = {
                "count": delta_count,
                "total_s": round(delta_total, 3),
                "avg_s": round(delta_total / delta_count, 3) if delta_count else 0.0,
            }
        return dict(sorted(result.items()))

    @staticmethod
    def _card_from_dict(value: dict) -> XhsNoteCard:
        return XhsNoteCard(
            note_id=str(value.get("note_id") or ""),
            title=str(value.get("title") or ""),
            author=str(value.get("author") or ""),
            author_id=str(value.get("author_id") or ""),
            author_url=str(value.get("author_url") or ""),
            likes=str(value.get("likes") or ""),
            link=str(value.get("link") or ""),
            cover_url=str(value.get("cover_url") or ""),
            type=str(value.get("type") or ""),
            position=int(value.get("position") or 0),
            xsec_token=str(value.get("xsec_token") or ""),
        )

    def _select_cards(self, cards: list[XhsNoteCard], *, total_limit: int) -> list[XhsNoteCard]:
        selected: list[XhsNoteCard] = []
        seen: set[str] = set()
        for card in cards:
            key = card.note_id or card.link or f"pos:{card.position}"
            if not key or key in seen:
                continue
            seen.add(key)
            selected.append(card)
            if len(selected) >= total_limit:
                break
        return selected


# ── transient tools (no artifact, results are control-flow signals) ──────────


class XhsPageStateTool(XhsToolBase):
    name = "xhs_page_state"
    description = "Detect the current Xiaohongshu page state: homepage, search_results, note_detail, or profile_page."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        return _json(await self.runtime.detect_state())


class XhsCollectCarouselImagesTool(XhsToolBase):
    name = "xhs_collect_carousel_images"
    description = "Collect image URLs from the currently open Xiaohongshu image note carousel."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        urls = await self.runtime.collect_carousel_images()
        return _json({"ok": True, "count": len(urls), "image_urls": urls})


class XhsScrollInNoteTool(XhsToolBase):
    name = "xhs_scroll_in_note"
    description = (
        "Scroll inside the currently open Xiaohongshu note modal to trigger lazy-loaded "
        "comments. Call this between xhs_extract_comments invocations to load more."
    )

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        result = await self.runtime.scroll_in_note(pixels=600)
        return _json({"ok": bool(result.get("ok")), **{k: result.get(k) for k in ("container", "delta", "error")}})


class XhsListSearchTabsTool(XhsToolBase):
    name = "xhs_list_search_tabs"
    description = "List Xiaohongshu search-result category tabs (全部/图文/视频/用户) with which one is active."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        tabs = await self.runtime.list_search_tabs()
        return _json({"ok": True, "count": len(tabs), "tabs": tabs})


class XhsClickSearchTabTool(XhsToolBase):
    name = "xhs_click_search_tab"
    description = (
        "Switch the Xiaohongshu search-result category by clicking a tab "
        "(one of: 全部, 图文, 视频, 用户)."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {"label": {"type": "string", "enum": ["全部", "图文", "视频", "用户"]}},
            "required": ["label"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        return _json(await self.runtime.click_search_tab(str(params["label"])))


class XhsCloseNoteTool(XhsToolBase):
    name = "xhs_close_note"
    description = (
        "Close the currently open Xiaohongshu note modal using human-like inputs (Escape first, "
        "fallback to JS-dispatched Escape, fallback to clicking the close X). Use between "
        "successive note reads instead of reloading the search page."
    )

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        result = await self.runtime.close_note()
        return _json(result)


def build_xhs_tools(runtime: XhsRuntime) -> list[Tool]:
    return [
        XhsSearchNotesTool(runtime),
        XhsTopicScanTool(runtime),
        XhsListSearchTabsTool(runtime),
        XhsClickSearchTabTool(runtime),
        XhsPageStateTool(runtime),
        XhsExtractSearchCardsTool(runtime),
        XhsReadNoteTool(runtime),
        XhsExtractNoteTool(runtime),
        XhsExtractCommentsTool(runtime),
        XhsExtractProfileTool(runtime),
        XhsCollectCarouselImagesTool(runtime),
        XhsScrollInNoteTool(runtime),
        XhsCloseNoteTool(runtime),
    ]
