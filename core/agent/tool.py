"""Minimal tool interface for the Socai agent loop.

Each tool exposes:
- name / description / parameters  → sent to the Anthropic tool_use API
- execute()                        → called when the LLM picks this tool
"""

from __future__ import annotations

import json
from abc import ABC, abstractmethod
from dataclasses import dataclass
from pathlib import Path
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from .run_state import RunState


@dataclass
class ToolContext:
    """Shared runtime state available to every tool invocation."""

    run_dir: Path
    run_state: RunState | None = None
    screenshot_counter: int = 0
    screenshot_max_dim: int = 0  # 0 = no downscaling
    artifact_counter: int = 0
    turn: int = 0
    active_tool_name: str = ""

    def next_screenshot_path(self, label: str = "screenshot") -> Path:
        self.screenshot_counter += 1
        path = self.run_dir / f"{self.screenshot_counter:03d}_{label}.png"
        path.parent.mkdir(parents=True, exist_ok=True)
        return path

    def next_artifact_path(
        self,
        label: str = "artifact",
        *,
        suffix: str = ".json",
        subdir: str = "artifacts",
    ) -> Path:
        self.artifact_counter += 1
        safe_label = "".join(ch if ch.isalnum() or ch in {"_", "-"} else "_" for ch in label).strip("_")
        safe_label = safe_label or "artifact"
        directory = self.run_dir / subdir
        directory.mkdir(parents=True, exist_ok=True)
        return directory / f"{self.artifact_counter:03d}_{safe_label}{suffix}"

    def register_artifact(
        self,
        path: Path,
        *,
        label: str = "",
        artifact_kind: str = "",
        summary: str = "",
        metadata: dict | None = None,
        payload=None,
        source_tool: str = "",
    ) -> str:
        rel = str(path.relative_to(self.run_dir))
        if self.run_state is not None:
            self.run_state.record_artifact(
                rel,
                label=label,
                artifact_kind=artifact_kind,
                source_tool=source_tool or self.active_tool_name,
                turn=self.turn or None,
                summary=summary,
                payload=payload,
                metadata=metadata or {},
            )
        return rel

    def write_json_artifact(
        self,
        label: str,
        payload: dict,
        *,
        subdir: str = "artifacts",
        source_tool: str = "",
        artifact_kind: str = "json",
        summary: str = "",
        metadata: dict | None = None,
    ) -> str:
        path = self.next_artifact_path(label, suffix=".json", subdir=subdir)
        path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")
        return self.register_artifact(
            path,
            label=label,
            artifact_kind=artifact_kind,
            summary=summary,
            metadata=metadata,
            payload=payload,
            source_tool=source_tool,
        )


class Tool(ABC):
    """Base class for agent tools."""

    @property
    def always_available(self) -> bool:
        return False

    @property
    @abstractmethod
    def name(self) -> str: ...

    @property
    @abstractmethod
    def description(self) -> str: ...

    @property
    @abstractmethod
    def parameters(self) -> dict:
        """JSON Schema object for the tool input (Anthropic format)."""
        ...

    @abstractmethod
    async def execute(self, params: dict, ctx: ToolContext) -> str | list:
        """Run the tool and return a result for the LLM.

        May return a plain string, or a list of Anthropic content blocks
        (e.g. [{"type": "text", ...}, {"type": "image", ...}]) when the
        tool wants to send images back to the model.
        """
        ...

    def to_api_schema(self) -> dict:
        """Format for the Anthropic ``tools`` parameter."""
        return {
            "name": self.name,
            "description": self.description,
            "input_schema": self.parameters,
        }
