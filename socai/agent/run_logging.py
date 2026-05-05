"""Persistent debug logging helpers for Socai agent runs."""

from __future__ import annotations

import json
import os
import traceback
from datetime import datetime
from pathlib import Path
from typing import Any


def default_runs_root() -> Path:
    return Path(os.environ.get("SOCAI_RUNS_DIR", ".socai/runs"))


def safe_slug(text: str, max_chars: int = 48) -> str:
    raw = str(text or "agent").strip().replace("/", " ")
    slug = "".join(ch if ch.isalnum() or ch in {"_", "-"} else "_" for ch in raw)
    slug = "_".join(part for part in slug.split("_") if part)
    return (slug or "agent")[:max_chars]


def make_run_dir(task: str) -> Path:
    """Return a new default run directory for a task."""

    ts = datetime.now().strftime("%Y%m%d_%H%M%S")
    base = default_runs_root() / f"agent_{ts}_{safe_slug(task)}"
    path = base
    suffix = 2
    while path.exists():
        path = base.with_name(f"{base.name}_{suffix}")
        suffix += 1
    return path


def timestamp() -> str:
    return datetime.now().isoformat()


def current_traceback() -> str:
    return traceback.format_exc()


def json_safe_for_log(value: Any, *, max_string_chars: int = 100_000) -> Any:
    """Convert arbitrary content blocks into JSON-safe debug payloads."""

    if isinstance(value, str):
        if len(value) <= max_string_chars:
            return value
        return value[:max_string_chars] + f"\n... [truncated {len(value) - max_string_chars} chars]"
    if isinstance(value, (int, float, bool)) or value is None:
        return value
    if isinstance(value, dict):
        if value.get("type") == "image":
            return {
                "type": "image",
                "omitted": True,
                "note": "Image data omitted from debug log; use the saved artifact path if present.",
            }
        return {str(k): json_safe_for_log(v, max_string_chars=max_string_chars) for k, v in value.items()}
    if isinstance(value, list):
        return [json_safe_for_log(item, max_string_chars=max_string_chars) for item in value]
    try:
        json.dumps(value)
        return value
    except TypeError:
        return str(value)


def write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(json_safe_for_log(payload), ensure_ascii=False, indent=2), encoding="utf-8")


def write_jsonl(path: Path, entry: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(json_safe_for_log(entry), ensure_ascii=False) + "\n")


class JsonlEventLogger:
    """Small wrapper for timestamped JSONL event files."""

    def __init__(self, path: Path):
        self.path = Path(path)

    def write(self, event_type: str, **payload: Any) -> None:
        write_jsonl(self.path, {"type": event_type, "timestamp": timestamp(), **payload})


class RunDebugLogger:
    """Owns debug files under a single agent run directory."""

    def __init__(self, run_dir: Path):
        self.run_dir = Path(run_dir)
        self.reasoning_log_path = self.run_dir / "reasoning_log.jsonl"
        self.tool_results_dir = self.run_dir / "tool_results"
        self.conversation_path = self.run_dir / "conversation.json"
        self.agent_log_path = self.run_dir / "agent_log.json"

    def event(self, event_type: str, **payload: Any) -> None:
        write_jsonl(self.reasoning_log_path, {"type": event_type, "timestamp": timestamp(), **payload})

    def api_error(self, *, turn: int, error: str, forced_summary: bool = False) -> None:
        payload: dict[str, Any] = {
            "turn": turn,
            "error": error,
            "traceback": current_traceback(),
        }
        if forced_summary:
            payload["forced_summary"] = True
        self.event("api_error", **payload)

    def tool_result(
        self,
        *,
        turn: int,
        sequence: int,
        tool_name: str,
        tool_input: dict[str, Any],
        content: list[dict[str, Any]],
        duration_s: float,
        result_summary: str,
        repeat_count: int,
        error: str = "",
        traceback_text: str = "",
    ) -> str:
        result_file = self._write_tool_result_file(
            turn=turn,
            sequence=sequence,
            tool_name=tool_name,
            payload={
                "type": "tool_result",
                "timestamp": timestamp(),
                "turn": turn,
                "sequence": sequence,
                "tool": tool_name,
                "input": tool_input,
                "duration_s": duration_s,
                "error": error,
                "traceback": traceback_text,
                "content": content,
            },
        )
        self.event(
            "tool_result",
            turn=turn,
            sequence=sequence,
            tool=tool_name,
            input=tool_input,
            duration_s=duration_s,
            result_summary=result_summary,
            result_file=result_file,
            error=error,
            repeat_count=repeat_count,
        )
        return result_file

    def write_conversation(self, *, system_prompt: str, messages: list[dict[str, Any]]) -> Path:
        write_json(self.conversation_path, {"system": system_prompt, "messages": messages})
        return self.conversation_path

    def write_agent_summary(self, summary: dict[str, Any]) -> None:
        write_json(self.agent_log_path, summary)

    def _write_tool_result_file(
        self,
        *,
        turn: int,
        sequence: int,
        tool_name: str,
        payload: dict[str, Any],
    ) -> str:
        self.tool_results_dir.mkdir(parents=True, exist_ok=True)
        path = self.tool_results_dir / f"{turn:03d}_{sequence:02d}_{safe_slug(tool_name, max_chars=32)}.json"
        write_json(path, payload)
        return str(path.relative_to(self.run_dir))
