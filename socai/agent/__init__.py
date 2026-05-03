"""Minimal LLM-driven agent core.

This package intentionally contains only the generic agent loop, backend
adapter protocol, tool protocol, and per-run state. Browser/CDP and site tools
are injected from outside this package.
"""

from .backends import Backend, LLMResponse, ToolCall, create_backend
from .loop import run_agent
from .run_state import RunState
from .tool import Tool, ToolContext

__all__ = [
    "Backend",
    "LLMResponse",
    "RunState",
    "Tool",
    "ToolCall",
    "ToolContext",
    "create_backend",
    "run_agent",
]

