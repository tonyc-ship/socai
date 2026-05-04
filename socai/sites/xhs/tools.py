"""Agent tools for the Xiaohongshu site runtime.

Each tool here is a thin LLM-shape adapter over ``XhsRuntime``. The shared
``_emit`` helper handles three responsibilities common to every persistent
tool: tagging the payload with ``{site, action}``, writing it as a JSON
artifact under ``run_dir/site_results/``, and returning a truncated reply
for the model (full data lives on disk, agent context stays small).
"""

from __future__ import annotations

import json

from socai.agent.tool import Tool, ToolContext

from .runtime import XhsRuntime


def _json(payload: dict) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2)


class XhsToolBase(Tool):
    SITE = "xiaohongshu"

    def __init__(self, runtime: XhsRuntime):
        self.runtime = runtime

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
                "wait_seconds": {"type": "number", "default": 2.0},
            },
            "required": ["query"],
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        query = str(params.get("query") or "")
        payload = await self.runtime.search_notes(query, wait_seconds=float(params.get("wait_seconds", 2.0)))
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
        "body together with the top comments in one call."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "note_id": {"type": "string"},
                "index": {"type": "integer"},
                "with_comments": {"type": "boolean", "default": True},
                "max_comments": {"type": "integer", "default": 12},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        index = params.get("index")
        payload = await self.runtime.read_note(
            note_id=str(params.get("note_id") or ""),
            index=int(index) if index is not None else None,
            with_comments=bool(params.get("with_comments", True)),
            max_comments=int(params.get("max_comments", 12)),
        )
        entity = payload.get("entity") or {}
        comments = payload.get("comments") or []
        label = f"xhs_note_{entity.get('note_id') or 'note'}"
        return await self._emit(
            ctx,
            label=label,
            payload=payload,
            preview={"ok": payload.get("ok", False), "entity": entity, "comments": comments[:8]},
        )


class XhsExtractNoteTool(XhsToolBase):
    name = "xhs_extract_note"
    description = "Extract a Xiaohongshu note entity from the currently open note page (no comments)."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        note = await self.runtime.extract_note()
        payload = {"ok": True, "entity": note.to_dict()}
        return await self._emit(ctx, label=f"xhs_note_{note.note_id or 'current'}", payload=payload)


class XhsExtractCommentsTool(XhsToolBase):
    name = "xhs_extract_comments"
    description = (
        "Extract comments from the currently open Xiaohongshu note. Top-level comments include "
        "nested sub_comments. Sorted by heat (likes + replies + pinned bonus) by default."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "max_comments": {"type": "integer", "default": 12},
                "prefer_hot": {"type": "boolean", "default": True},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        comments = await self.runtime.extract_comments(
            prefer_hot=bool(params.get("prefer_hot", True)),
            max_comments=int(params.get("max_comments", 12)),
        )
        payload = {"ok": True, "count": len(comments), "comments": comments}
        return await self._emit(
            ctx,
            label="xhs_comments",
            payload=payload,
            preview={"ok": True, "count": len(comments), "comments": comments[:8]},
        )


# ── transient tools (no artifact, results are control-flow signals) ──────────


class XhsScrollInNoteTool(XhsToolBase):
    name = "xhs_scroll_in_note"
    description = (
        "Scroll inside the currently open Xiaohongshu note modal to trigger lazy-loaded "
        "comments. Call this between xhs_extract_comments invocations to load more."
    )

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {"pixels": {"type": "integer", "default": 600}},
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        result = await self.runtime.scroll_in_note(pixels=int(params.get("pixels", 600)))
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
        XhsListSearchTabsTool(runtime),
        XhsClickSearchTabTool(runtime),
        XhsExtractSearchCardsTool(runtime),
        XhsReadNoteTool(runtime),
        XhsExtractNoteTool(runtime),
        XhsExtractCommentsTool(runtime),
        XhsScrollInNoteTool(runtime),
        XhsCloseNoteTool(runtime),
    ]
