"""Structured per-run state for the generic Socai agent."""

from __future__ import annotations

import json
from copy import deepcopy
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit, urlunsplit


def _utc_now() -> str:
    return datetime.now(timezone.utc).isoformat()


def _truncate(text: str, max_chars: int) -> str:
    value = str(text or "").strip()
    if len(value) <= max_chars:
        return value
    return value[:max_chars] + "... [truncated]"


def _compact_value(value: Any, *, depth: int = 0) -> Any:
    if depth >= 3:
        if isinstance(value, str):
            return _truncate(value, 320)
        return value
    if isinstance(value, dict):
        compacted: dict[str, Any] = {}
        preferred = [
            "id",
            "entity_id",
            "note_id",
            "type",
            "entity_type",
            "title",
            "author",
            "url",
            "resolved_url",
            "summary",
            "content_summary",
            "key_points",
            "top_comments",
            "likes",
            "comments_count",
            "favorites",
            "screenshot",
            "artifact_path",
        ]
        keys = [key for key in preferred if key in value]
        keys.extend(key for key in value if key not in keys)
        for key in keys[:20]:
            compacted[str(key)] = _compact_value(value[key], depth=depth + 1)
        return compacted
    if isinstance(value, list):
        return [_compact_value(item, depth=depth + 1) for item in value[:8]]
    if isinstance(value, str):
        return _truncate(value, 600)
    return value


def _normalize_url(url: str) -> str:
    value = str(url or "").strip()
    if not value:
        return ""
    parts = urlsplit(value)
    return urlunsplit((parts.scheme, parts.netloc, parts.path, parts.query, ""))


def _looks_like_entity(value: Any) -> bool:
    if not isinstance(value, dict):
        return False
    interesting = {
        "id",
        "entity_id",
        "note_id",
        "url",
        "resolved_url",
        "title",
        "author",
        "content",
        "content_summary",
        "summary",
        "screenshot",
    }
    return any(key in value and value.get(key) not in (None, "", [], {}) for key in interesting)


def _iter_entity_candidates(payload: Any) -> list[dict[str, Any]]:
    candidates: list[dict[str, Any]] = []
    seen_ids: set[int] = set()

    def add_candidate(candidate: Any) -> None:
        if not _looks_like_entity(candidate):
            return
        marker = id(candidate)
        if marker in seen_ids:
            return
        seen_ids.add(marker)
        candidates.append(candidate)

    def walk(node: Any, *, depth: int = 0) -> None:
        if depth > 2:
            return
        if isinstance(node, dict):
            add_candidate(node)
            entity = node.get("entity")
            if isinstance(entity, dict):
                add_candidate(entity)
            for key in ("notes", "items", "results", "entities", "cards"):
                value = node.get(key)
                if isinstance(value, list):
                    for item in value[:20]:
                        if isinstance(item, dict) and isinstance(item.get("entity"), dict):
                            add_candidate(item["entity"])
                        else:
                            add_candidate(item)
        elif isinstance(node, list):
            for item in node[:20]:
                walk(item, depth=depth + 1)

    walk(payload)
    return candidates


def _entity_key(entity: dict[str, Any], artifact_path: str) -> str:
    for key in ("entity_id", "note_id", "id"):
        value = str(entity.get(key) or "").strip()
        if value:
            return f"id:{value}"
    url = _normalize_url(str(entity.get("url") or entity.get("resolved_url") or ""))
    if url:
        return f"url:{url}"
    title = str(entity.get("title") or "").strip().lower()
    author = str(entity.get("author") or "").strip().lower()
    if title and author:
        return f"title:{title}|author:{author}"
    if title:
        return f"title:{title}"
    return f"artifact:{artifact_path}"


def _merge_text(existing: str, incoming: str) -> str:
    existing = str(existing or "").strip()
    incoming = str(incoming or "").strip()
    if not incoming:
        return existing
    if not existing:
        return incoming
    return incoming if len(incoming) > len(existing) else existing


def _merge_unique_strings(existing: list[str], additions: list[str]) -> list[str]:
    merged: list[str] = []
    seen: set[str] = set()
    for item in [*(existing or []), *(additions or [])]:
        value = str(item or "").strip()
        if not value or value in seen:
            continue
        seen.add(value)
        merged.append(value)
    return merged


class RunState:
    """Persistent structured state for one agent run."""

    def __init__(self, run_dir: Path, task: str, *, model: str = "") -> None:
        self.run_dir = Path(run_dir)
        self.task = str(task or "").strip()
        self.model = str(model or "").strip()
        self.state_dir = self.run_dir / "run_state"
        self.state_dir.mkdir(parents=True, exist_ok=True)

        self.task_path = self.state_dir / "task.json"
        self.plan_path = self.state_dir / "plan.json"
        self.artifacts_path = self.state_dir / "artifacts.json"
        self.evidence_path = self.state_dir / "evidence.json"
        self.events_path = self.state_dir / "events.jsonl"
        self.working_memory_path = self.state_dir / "working_memory.md"

        self._plan: dict[str, Any] = {
            "task": self.task,
            "updated_at": _utc_now(),
            "steps": [],
            "notes": [],
        }
        self._artifacts: dict[str, dict[str, Any]] = {}
        self._evidence: dict[str, dict[str, Any]] = {}
        self._recent_events: list[dict[str, Any]] = []
        self._max_recent_events = 80

        self._write_json(
            self.task_path,
            {
                "task": self.task,
                "model": self.model,
                "created_at": _utc_now(),
                "run_dir": str(self.run_dir),
            },
        )
        self._flush_plan()
        self._flush_artifacts()
        self._flush_evidence()
        self._flush_working_memory()

    def _write_json(self, path: Path, payload: Any) -> None:
        path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")

    def _append_event(self, event: dict[str, Any]) -> None:
        enriched = {"timestamp": _utc_now(), **event}
        with self.events_path.open("a", encoding="utf-8") as handle:
            handle.write(json.dumps(enriched, ensure_ascii=False) + "\n")
        self._recent_events.append(enriched)
        if len(self._recent_events) > self._max_recent_events:
            self._recent_events = self._recent_events[-self._max_recent_events :]

    def _flush_plan(self) -> None:
        self._plan["updated_at"] = _utc_now()
        self._write_json(self.plan_path, self._plan)

    def _flush_artifacts(self) -> None:
        items = [self._artifacts[key] for key in sorted(self._artifacts)]
        self._write_json(self.artifacts_path, {"count": len(items), "items": items})

    def _flush_evidence(self) -> None:
        items = [self._evidence[key] for key in sorted(self._evidence)]
        self._write_json(self.evidence_path, {"count": len(items), "items": items})

    def _flush_working_memory(self) -> None:
        self.working_memory_path.write_text(self.render_working_memory(), encoding="utf-8")

    def note_assistant_turn(
        self,
        *,
        turn: int,
        text: str,
        tool_calls: list[dict[str, Any]] | None = None,
    ) -> None:
        self._append_event(
            {
                "type": "assistant_turn",
                "turn": turn,
                "text": _truncate(text, 800),
                "tool_calls": deepcopy(tool_calls or []),
            }
        )
        self._flush_working_memory()

    def note_tool_call(self, *, turn: int, tool_name: str, tool_input: dict[str, Any]) -> None:
        self._append_event(
            {
                "type": "tool_call",
                "turn": turn,
                "tool": tool_name,
                "input": _compact_value(tool_input),
            }
        )
        self._flush_working_memory()

    def note_tool_result(
        self,
        *,
        turn: int,
        tool_name: str,
        tool_input: dict[str, Any],
        result_summary: str,
        duration_s: float,
    ) -> None:
        self._append_event(
            {
                "type": "tool_result",
                "turn": turn,
                "tool": tool_name,
                "input": _compact_value(tool_input),
                "result_summary": _truncate(result_summary, 1200),
                "duration_s": round(float(duration_s), 2),
            }
        )
        self._flush_working_memory()

    def update_plan(
        self,
        steps: list[dict[str, Any]],
        *,
        note: str = "",
        turn: int | None = None,
    ) -> dict[str, Any]:
        normalized_steps: list[dict[str, Any]] = []
        for index, step in enumerate(steps):
            if not isinstance(step, dict):
                continue
            title = str(step.get("title") or step.get("step") or "").strip()
            if not title:
                continue
            status = str(step.get("status") or "pending").strip().lower()
            if status not in {"pending", "in_progress", "completed"}:
                status = "pending"
            normalized_steps.append(
                {
                    "id": str(step.get("id") or f"step_{index + 1}"),
                    "title": title,
                    "status": status,
                    "details": _truncate(str(step.get("details") or step.get("note") or ""), 400),
                }
            )
        self._plan["steps"] = normalized_steps
        if note:
            notes = self._plan.setdefault("notes", [])
            notes.append({"timestamp": _utc_now(), "turn": turn, "text": _truncate(note, 500)})
            self._plan["notes"] = notes[-20:]
        self._flush_plan()
        self._flush_working_memory()
        self._append_event(
            {
                "type": "plan_update",
                "turn": turn,
                "steps": deepcopy(normalized_steps),
                "note": _truncate(note, 500),
            }
        )
        return deepcopy(self._plan)

    def record_artifact(
        self,
        relative_path: str,
        *,
        label: str = "",
        artifact_kind: str = "",
        source_tool: str = "",
        turn: int | None = None,
        summary: str = "",
        payload: Any = None,
        metadata: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        rel = str(relative_path or "").strip().lstrip("./")
        if not rel:
            raise ValueError("relative_path is required")
        path = self.run_dir / rel
        entry = self._artifacts.get(rel, {})
        artifact = {
            "path": rel,
            "label": label or entry.get("label") or Path(rel).stem,
            "kind": artifact_kind or entry.get("kind") or path.suffix.lstrip(".") or "file",
            "source_tool": source_tool or entry.get("source_tool") or "",
            "turn": turn if turn is not None else entry.get("turn"),
            "summary": _merge_text(entry.get("summary", ""), summary),
            "size_bytes": path.stat().st_size if path.exists() and path.is_file() else entry.get("size_bytes"),
            "updated_at": _utc_now(),
            "metadata": {**(entry.get("metadata") or {}), **(metadata or {})},
        }
        if payload is not None:
            artifact["preview"] = _compact_value(payload)
        elif "preview" in entry:
            artifact["preview"] = entry["preview"]
        self._artifacts[rel] = artifact
        if payload is not None:
            self._ingest_evidence(payload, artifact_path=rel, source_tool=source_tool, turn=turn)
        self._append_event(
            {
                "type": "artifact_recorded",
                "turn": turn,
                "path": rel,
                "kind": artifact["kind"],
                "source_tool": artifact["source_tool"],
            }
        )
        self._flush_artifacts()
        self._flush_evidence()
        self._flush_working_memory()
        return deepcopy(artifact)

    def _ingest_evidence(
        self,
        payload: Any,
        *,
        artifact_path: str,
        source_tool: str,
        turn: int | None,
    ) -> None:
        for entity in _iter_entity_candidates(payload):
            key = _entity_key(entity, artifact_path)
            existing = self._evidence.get(key, {})
            summary = str(
                entity.get("content_summary")
                or entity.get("summary")
                or entity.get("content")
                or entity.get("title")
                or ""
            )
            record = {
                "key": key,
                "kind": str(entity.get("entity_type") or entity.get("type") or existing.get("kind") or "entity"),
                "title": str(entity.get("title") or existing.get("title") or "").strip(),
                "author": str(entity.get("author") or existing.get("author") or "").strip(),
                "url": _normalize_url(
                    str(entity.get("url") or entity.get("resolved_url") or existing.get("url") or "")
                ),
                "screenshot": str(entity.get("screenshot") or existing.get("screenshot") or "").strip(),
                "summary": _merge_text(existing.get("summary", ""), _truncate(summary, 1200)),
                "artifact_paths": _merge_unique_strings(existing.get("artifact_paths", []), [artifact_path]),
                "source_tools": _merge_unique_strings(existing.get("source_tools", []), [source_tool]),
                "counts": {
                    "likes": entity.get("likes") or (existing.get("counts") or {}).get("likes", ""),
                    "favorites": entity.get("favorites") or (existing.get("counts") or {}).get("favorites", ""),
                    "comments_count": entity.get("comments_count")
                    or (existing.get("counts") or {}).get("comments_count", ""),
                },
                "top_comments": _compact_value(entity.get("top_comments") or (existing.get("top_comments") or [])),
                "key_points": _compact_value(entity.get("key_points") or (existing.get("key_points") or [])),
                "snapshot": _compact_value(entity),
                "updated_at": _utc_now(),
                "turn": turn if turn is not None else existing.get("turn"),
            }
            self._evidence[key] = record

    def render_working_memory(self, *, max_recent_events: int = 10, max_evidence: int = 8) -> str:
        recent_events = self._recent_events[-max_recent_events:]
        lines = ["# Task", self.task or "(empty task)", "", "# Plan"]
        steps = self._plan.get("steps") or []
        if steps:
            for step in steps:
                status = str(step.get("status") or "pending")
                title = str(step.get("title") or "").strip()
                details = str(step.get("details") or "").strip()
                line = f"- [{status}] {title}"
                if details:
                    line += f" — {details}"
                lines.append(line)
        else:
            lines.append("- No explicit plan has been recorded yet.")
        lines.extend(
            [
                "",
                "# Current State",
                f"- Saved artifacts: {len(self._artifacts)}",
                f"- Evidence records: {len(self._evidence)}",
            ]
        )
        latest_result = next(
            (event for event in reversed(recent_events) if event.get("type") == "tool_result"),
            None,
        )
        if latest_result:
            lines.append(
                f"- Latest tool result: turn {latest_result.get('turn')} "
                f"{latest_result.get('tool')} — {_truncate(latest_result.get('result_summary', ''), 200)}"
            )
        lines.extend(["", "# Recent Activity"])
        if recent_events:
            for event in recent_events:
                event_type = event.get("type")
                if event_type == "assistant_turn":
                    lines.append(f"- turn {event.get('turn')} assistant: {_truncate(event.get('text', ''), 160)}")
                elif event_type == "tool_call":
                    lines.append(f"- turn {event.get('turn')} tool_call {event.get('tool')}")
                elif event_type == "tool_result":
                    lines.append(
                        f"- turn {event.get('turn')} tool_result {event.get('tool')}: "
                        f"{_truncate(event.get('result_summary', ''), 160)}"
                    )
                elif event_type == "plan_update":
                    lines.append(f"- turn {event.get('turn')} plan updated")
                elif event_type == "artifact_recorded":
                    lines.append(f"- turn {event.get('turn')} saved {event.get('path')}")
        else:
            lines.append("- No activity has been recorded yet.")
        lines.extend(["", "# Key Evidence"])
        evidence_items = list(self._evidence.values())[-max_evidence:]
        if evidence_items:
            for item in evidence_items:
                title = str(item.get("title") or item.get("key") or "").strip()
                author = str(item.get("author") or "").strip()
                summary = str(item.get("summary") or "").strip()
                suffix = f" — {author}" if author else ""
                details = f" | {_truncate(summary, 180)}" if summary else ""
                lines.append(f"- {title}{suffix}{details}")
        else:
            lines.append("- No structured evidence has been extracted yet.")
        lines.extend(
            [
                "",
                "# Retrieval",
                "- Use a task-plan tool, if one is registered, to keep a live checklist for complex tasks.",
                "- Use run-state or artifact-reading tools, if registered by the host, to revisit earlier findings.",
                "",
            ]
        )
        return "\n".join(lines).rstrip() + "\n"

    def context_block(self, *, max_chars: int = 6000) -> str:
        return _truncate(self.render_working_memory(), max_chars)

    def has_structured_state(self) -> bool:
        if self._plan.get("steps") or self._evidence:
            return True
        for artifact in self._artifacts.values():
            kind = str(artifact.get("kind") or "").strip().lower()
            metadata = artifact.get("metadata") or {}
            if kind == "image" and metadata.get("category") == "screenshot":
                continue
            return True
        return False

    def report_grounding_context(
        self,
        *,
        max_evidence: int = 10,
        max_artifacts: int = 10,
        max_chars: int = 5000,
    ) -> str:
        lines = ["# Task", self.task or "(empty task)", "", "# Plan"]
        steps = self._plan.get("steps") or []
        if steps:
            for step in steps:
                lines.append(f"- [{step.get('status')}] {step.get('title')}")
        else:
            lines.append("- No explicit plan recorded.")

        lines.extend(["", "# Evidence"])
        evidence_items = list(self._evidence.values())[-max_evidence:]
        if evidence_items:
            for item in evidence_items:
                title = str(item.get("title") or item.get("key") or "").strip()
                author = str(item.get("author") or "").strip()
                counts = item.get("counts") or {}
                signals = ", ".join(
                    f"{name}={value}"
                    for name, value in (
                        ("likes", counts.get("likes")),
                        ("favorites", counts.get("favorites")),
                        ("comments", counts.get("comments_count")),
                    )
                    if value
                )
                summary = str(item.get("summary") or "").strip()
                screenshot = str(item.get("screenshot") or "").strip()
                line = f"- {title}"
                if author:
                    line += f" — {author}"
                if signals:
                    line += f" | {signals}"
                if screenshot:
                    line += f" | screenshot={screenshot}"
                lines.append(line)
                if summary:
                    lines.append(f"  summary: {_truncate(summary, 240)}")
        else:
            lines.append("- No structured evidence extracted yet.")

        lines.extend(["", "# Saved Artifacts"])
        artifact_items = list(self._artifacts.values())[-max_artifacts:]
        if artifact_items:
            for item in artifact_items:
                line = f"- {item.get('path')}"
                if item.get("kind"):
                    line += f" ({item.get('kind')})"
                if item.get("summary"):
                    line += f" — {_truncate(str(item.get('summary') or ''), 140)}"
                lines.append(line)
        else:
            lines.append("- No saved artifacts.")

        return _truncate("\n".join(lines).rstrip() + "\n", max_chars)

    def render_markdown_appendix(
        self,
        report: str,
        *,
        max_evidence: int = 12,
    ) -> str:
        base = (report or "").rstrip()
        sections: list[str] = []

        steps = self._plan.get("steps") or []
        if steps and "## Task Checklist" not in base:
            lines = ["## Task Checklist", ""]
            for step in steps:
                lines.append(f"- [{step.get('status')}] {step.get('title')}")
            sections.append("\n".join(lines).rstrip())

        evidence_items = list(self._evidence.values())[-max_evidence:]
        if evidence_items and "## Structured Evidence Ledger" not in base:
            lines = [
                "## Structured Evidence Ledger",
                "",
                "| Item | Type | Signals | Evidence |",
                "| --- | --- | --- | --- |",
            ]
            details: list[str] = []
            for item in evidence_items:
                title = str(item.get("title") or item.get("key") or "").strip() or "(untitled)"
                author = str(item.get("author") or "").strip()
                label = f"{title} - {author}" if author else title
                kind = str(item.get("kind") or "")
                counts = item.get("counts") or {}
                signals = ", ".join(
                    f"{name}={value}"
                    for name, value in (
                        ("likes", counts.get("likes")),
                        ("favorites", counts.get("favorites")),
                        ("comments", counts.get("comments_count")),
                    )
                    if value
                ) or "-"
                evidence_parts = []
                url = str(item.get("url") or "").strip()
                screenshot = str(item.get("screenshot") or "").strip()
                if url:
                    evidence_parts.append(f"[link]({url})")
                if screenshot:
                    evidence_parts.append(f"`{screenshot}`")
                if not evidence_parts:
                    evidence_parts.append("-")
                lines.append(
                    "| "
                    + " | ".join(
                        [
                            label.replace("|", "/"),
                            kind.replace("|", "/") or "-",
                            signals.replace("|", "/"),
                            " ".join(evidence_parts).replace("|", "/"),
                        ]
                    )
                    + " |"
                )

                summary = str(item.get("summary") or "").strip()
                key_points = item.get("key_points") or []
                top_comments = item.get("top_comments") or []
                detail_lines = [f"### {label}"]
                if summary:
                    detail_lines.append(f"- Summary: {_truncate(summary, 280)}")
                if key_points:
                    detail_lines.append(
                        "- Key Points: " + "; ".join(_truncate(str(point), 80) for point in key_points[:4])
                    )
                if top_comments:
                    comments = []
                    for comment in top_comments[:2]:
                        if isinstance(comment, dict):
                            comments.append(_truncate(str(comment.get("text") or comment.get("content") or ""), 80))
                        else:
                            comments.append(_truncate(str(comment), 80))
                    comments = [comment for comment in comments if comment]
                    if comments:
                        detail_lines.append("- Top Comments: " + "; ".join(comments))
                if screenshot:
                    detail_lines.append(f"- Screenshot: `{screenshot}`")
                details.append("\n".join(detail_lines))
            sections.append("\n".join(lines).rstrip())
            if details:
                sections.append("\n\n".join(details).rstrip())

        if not sections:
            return base + ("\n" if report.endswith("\n") else "")
        if not base:
            return "\n\n".join(sections).rstrip() + "\n"
        return base + "\n\n" + "\n\n".join(section.rstrip() for section in sections) + "\n"

    def read_section(
        self,
        section: str,
        *,
        item_key: str = "",
        limit: int = 10,
    ) -> dict[str, Any]:
        normalized = str(section or "").strip().lower()
        if normalized == "working_memory":
            return {
                "section": "working_memory",
                "content": self.working_memory_path.read_text(encoding="utf-8") if self.working_memory_path.exists() else "",
            }
        if normalized == "plan":
            return {"section": "plan", "content": deepcopy(self._plan)}
        if normalized == "artifacts":
            if item_key:
                return {"section": "artifacts", "item": deepcopy(self._artifacts.get(item_key))}
            items = [self._artifacts[key] for key in sorted(self._artifacts)]
            return {"section": "artifacts", "count": len(items), "items": deepcopy(items[-max(1, limit) :])}
        if normalized == "evidence":
            if item_key:
                return {"section": "evidence", "item": deepcopy(self._evidence.get(item_key))}
            items = [self._evidence[key] for key in sorted(self._evidence)]
            return {"section": "evidence", "count": len(items), "items": deepcopy(items[-max(1, limit) :])}
        if normalized == "events":
            return {
                "section": "events",
                "count": len(self._recent_events),
                "items": deepcopy(self._recent_events[-max(1, limit) :]),
            }
        raise ValueError(f"Unknown run state section: {section}")

    def read_artifact(self, relative_path: str, *, max_chars: int = 20000) -> dict[str, Any]:
        rel = str(relative_path or "").strip().lstrip("./")
        if not rel:
            raise ValueError("Artifact path is required")
        path = self.run_dir / rel
        if not path.is_file():
            raise FileNotFoundError(rel)
        text = path.read_text(encoding="utf-8")
        truncated = len(text) > max_chars
        return {
            "path": rel,
            "truncated": truncated,
            "content": text[:max_chars] if truncated else text,
            "size_bytes": path.stat().st_size,
        }
