"""Project-local Xiaohongshu analysis history."""

from __future__ import annotations

import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def _now() -> str:
    return datetime.now(timezone.utc).isoformat()


class XhsHistoryStore:
    """Small JSON history store keyed by note_id.

    This is intentionally project-local, mirroring Socai's `.socai/runs`
    artifacts rather than reusing FlowLens state.
    """

    def __init__(self, path: str | Path = ".socai/xhs/history.json") -> None:
        self.path = Path(path)

    def _read(self) -> dict[str, Any]:
        if not self.path.exists():
            return {"notes": {}}
        try:
            value = json.loads(self.path.read_text(encoding="utf-8"))
        except Exception:
            return {"notes": {}}
        if not isinstance(value, dict):
            return {"notes": {}}
        notes = value.get("notes")
        if not isinstance(notes, dict):
            value["notes"] = {}
        return value

    def _write(self, data: dict[str, Any]) -> None:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        self.path.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")

    def get(self, note_id: str) -> dict[str, Any] | None:
        key = str(note_id or "").strip()
        if not key:
            return None
        item = self._read().get("notes", {}).get(key)
        return item if isinstance(item, dict) else None

    def annotate_card(self, card: dict[str, Any]) -> dict[str, Any]:
        out = dict(card)
        note_id = str(out.get("note_id") or "").strip()
        history = self.get(note_id)
        if history:
            out["already_analyzed"] = True
            out["history_level"] = history.get("level", "")
            out["history_include_media"] = bool(history.get("include_media", False))
        return out

    def record_note(
        self,
        entity: dict[str, Any],
        *,
        artifact: str = "",
        run_dir: str = "",
        level: str = "",
        include_media: bool = False,
        source_tool: str = "",
    ) -> None:
        note_id = str(entity.get("note_id") or "").strip()
        if not note_id:
            return
        data = self._read()
        notes = data.setdefault("notes", {})
        previous = notes.get(note_id) if isinstance(notes.get(note_id), dict) else {}
        count = int(previous.get("analysis_count") or 0) + 1
        notes[note_id] = {
            **previous,
            "note_id": note_id,
            "title": str(entity.get("title") or previous.get("title") or ""),
            "author": str(entity.get("author") or previous.get("author") or ""),
            "url": str(entity.get("url") or previous.get("url") or ""),
            "artifact": artifact or previous.get("artifact", ""),
            "run_dir": run_dir or previous.get("run_dir", ""),
            "level": level or entity.get("extraction_level") or previous.get("level", ""),
            "include_media": bool(include_media or previous.get("include_media", False)),
            "source_tool": source_tool or previous.get("source_tool", ""),
            "analysis_count": count,
            "first_seen_at": previous.get("first_seen_at") or _now(),
            "last_seen_at": _now(),
        }
        self._write(data)
