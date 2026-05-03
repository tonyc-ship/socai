"""Agent tools for the Xiaohongshu site runtime."""

from __future__ import annotations

import json

from socai.agent.tool import Tool, ToolContext

from .runtime import XhsRuntime


def _json(payload: dict) -> str:
    return json.dumps(payload, ensure_ascii=False, indent=2)


class XhsToolBase(Tool):
    def __init__(self, runtime: XhsRuntime):
        self.runtime = runtime

    def _record(self, ctx: ToolContext, label: str, payload: dict) -> str:
        return ctx.write_json_artifact(
            label,
            payload,
            subdir="site_results",
            source_tool=self.name,
            artifact_kind="site_result",
            summary=str(payload.get("summary") or payload.get("action") or label),
            metadata={"site": "xiaohongshu"},
        )


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
        payload = await self.runtime.search_notes(
            str(params.get("query") or ""),
            wait_seconds=float(params.get("wait_seconds", 2.0)),
        )
        payload.update({"site": "xiaohongshu", "action": self.name})
        artifact = self._record(ctx, f"xhs_search_{params.get('query') or 'query'}", payload)
        return _json({"ok": payload["ok"], "count": payload["count"], "cards": payload["cards"][:8], "artifact": artifact})


class XhsExtractSearchCardsTool(XhsToolBase):
    name = "xhs_extract_search_cards"
    description = "Extract Xiaohongshu note cards from the current search/profile grid without navigating."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        cards = await self.runtime.extract_search_cards()
        payload = {
            "site": "xiaohongshu",
            "action": self.name,
            "ok": True,
            "count": len(cards),
            "cards": [card.to_dict() for card in cards],
        }
        artifact = self._record(ctx, "xhs_search_cards", payload)
        return _json({"ok": True, "count": len(cards), "cards": payload["cards"][:8], "artifact": artifact})


class XhsReadNoteTool(XhsToolBase):
    name = "xhs_read_note"
    description = "Open a Xiaohongshu note by note_id or index from current cards, then extract a minimal note entity."

    @property
    def parameters(self) -> dict:
        return {
            "type": "object",
            "properties": {
                "note_id": {"type": "string"},
                "index": {"type": "integer"},
            },
        }

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        index = params.get("index")
        payload = await self.runtime.read_note(
            note_id=str(params.get("note_id") or ""),
            index=int(index) if index is not None else None,
        )
        payload.update({"site": "xiaohongshu", "action": self.name})
        note_id = payload.get("entity", {}).get("note_id") or "note"
        artifact = self._record(ctx, f"xhs_note_{note_id}", payload)
        return _json({"ok": True, "entity": payload["entity"], "artifact": artifact})


class XhsExtractNoteTool(XhsToolBase):
    name = "xhs_extract_note"
    description = "Extract a minimal Xiaohongshu note entity from the currently open note page."

    @property
    def parameters(self) -> dict:
        return {"type": "object", "properties": {}}

    async def execute(self, params: dict, ctx: ToolContext) -> str:
        note = await self.runtime.extract_note()
        payload = {"site": "xiaohongshu", "action": self.name, "ok": True, "entity": note.to_dict()}
        artifact = self._record(ctx, f"xhs_note_{note.note_id or 'current'}", payload)
        return _json({"ok": True, "entity": payload["entity"], "artifact": artifact})


def build_xhs_tools(runtime: XhsRuntime) -> list[Tool]:
    return [
        XhsSearchNotesTool(runtime),
        XhsExtractSearchCardsTool(runtime),
        XhsReadNoteTool(runtime),
        XhsExtractNoteTool(runtime),
    ]
