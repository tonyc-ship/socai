"""Minimal agent loop.

This module is intentionally browser-agnostic. CDP/browser tools should be
registered as ``Tool`` instances by a higher layer.
"""

from __future__ import annotations

import asyncio
import hashlib
import json
import os
import time
from datetime import datetime
from pathlib import Path

from .backends import Backend, create_backend
from .run_state import RunState
from .tool import Tool, ToolContext


_BASE_SYSTEM_PROMPT = """\
You are a computer-use agent. Use the provided tools when they help complete
the user's task. Think briefly, take one or more useful actions, verify results
from tool output, and finish with a concise report when the task is complete.

Rules:
- Prefer high-level task/site tools over low-level manual actions when both exist.
- Do not invent observations. Use tool results as evidence.
- If a tool fails, explain the failure and choose a smaller recovery step.
- When enough evidence has been collected, stop calling tools and answer.
"""


def _default_runs_root() -> Path:
    return Path(os.environ.get("SOCAI_RUNS_DIR", ".socai/runs"))


def _safe_slug(text: str, max_chars: int = 48) -> str:
    raw = str(text or "agent").strip().replace("/", " ")
    slug = "".join(ch if ch.isalnum() or ch in {"_", "-"} else "_" for ch in raw)
    slug = "_".join(part for part in slug.split("_") if part)
    return (slug or "agent")[:max_chars]


def _build_system_prompt(tools: list[Tool], extra_instructions: str = "") -> str:
    parts = [_BASE_SYSTEM_PROMPT]
    if tools:
        parts.append(
            "Available tool names: "
            + ", ".join(f"`{tool.name}`" for tool in tools)
            + ". Tool schemas are provided separately."
        )
    if extra_instructions.strip():
        parts.append("Additional instructions:\n\n" + extra_instructions.strip())
    return "\n\n".join(parts)


def _tool_call_signature(tool_name: str, tool_input: dict) -> str:
    try:
        payload = json.dumps(tool_input or {}, sort_keys=True, ensure_ascii=False)
    except Exception:
        payload = str(tool_input)
    return hashlib.md5(f"{tool_name}::{payload}".encode("utf-8")).hexdigest()[:12]


def _text_summary(content_blocks: list[dict], max_len: int = 500) -> str:
    parts: list[str] = []
    for block in content_blocks:
        if block.get("type") == "text":
            parts.append(str(block.get("text", ""))[:max_len])
        elif block.get("type") == "image":
            parts.append("[image]")
    return " | ".join(parts)[:max_len]


def _compact_memory_entries(entries: list[str], max_chars: int) -> str:
    if not entries or max_chars <= 0:
        return ""
    selected: list[str] = []
    total = 0
    for entry in reversed(entries):
        entry = str(entry or "").strip()
        if not entry:
            continue
        projected = total + len(entry) + 1
        if selected and projected > max_chars:
            break
        selected.append(entry[:max_chars] if not selected and len(entry) > max_chars else entry)
        total = min(projected, max_chars)
        if total >= max_chars:
            break
    return "\n".join(reversed(selected))[:max_chars]


def _is_tool_result_message(message: dict) -> bool:
    content = message.get("content")
    if isinstance(content, str):
        return content.lstrip().startswith("[Tool result for ")
    if not isinstance(content, list):
        return False
    return any(
        isinstance(block, dict) and block.get("type") in {"tool_result", "function_call_output"}
        for block in content
    )


def _prepare_messages_for_context(
    messages: list[dict],
    run_state: RunState | None,
    memory_entries: list[str],
    *,
    keep_recent_messages: int = 12,
    memory_max_chars: int = 6000,
) -> list[dict]:
    if len(messages) <= keep_recent_messages + 2:
        return messages

    sections: list[str] = []
    if run_state is not None:
        state_block = run_state.context_block(max_chars=max(1200, memory_max_chars // 2))
        if state_block:
            sections.append("Structured run state from earlier turns:\n\n" + state_block)
    memory = _compact_memory_entries(memory_entries, memory_max_chars)
    if memory:
        sections.append("Condensed event memory from earlier turns:\n\n" + memory)
    if not sections:
        return messages

    recent = list(messages[-keep_recent_messages:])
    while recent and _is_tool_result_message(recent[0]):
        recent.pop(0)
    return [messages[0], {"role": "user", "content": "\n\n".join(sections)}, *recent]


async def _execute_tool(tool: Tool, params: dict, ctx: ToolContext) -> list[dict]:
    result = await tool.execute(params, ctx)
    if isinstance(result, str):
        return [{"type": "text", "text": result}]
    if isinstance(result, list):
        return result
    return [{"type": "text", "text": str(result)}]


def _write_jsonl(path: Path, entry: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(json.dumps(entry, ensure_ascii=False) + "\n")


async def run_agent(
    task: str,
    *,
    backend: Backend | None = None,
    tools: list[Tool] | None = None,
    run_dir: str | Path | None = None,
    max_turns: int = 30,
    model: str | None = None,
    extra_instructions: str = "",
    log_callback=None,
) -> dict:
    """Run the generic agent loop.

    Args:
        task: User task.
        backend: Optional prebuilt backend. Tests and app hosts can inject one.
        tools: Concrete tools available to the agent. Browser/CDP tools are
            intentionally supplied by the caller.
        run_dir: Output directory for report, logs, and run state.
        max_turns: Maximum model turns.
        model: Model id used when ``backend`` is not provided.
        extra_instructions: Additional system prompt text.
        log_callback: Optional ``callable(event, detail)`` for host logging.
    """

    if run_dir is None:
        ts = datetime.now().strftime("%Y%m%d_%H%M%S")
        run_dir = _default_runs_root() / f"agent_{ts}_{_safe_slug(task)}"
    run_path = Path(run_dir)
    run_path.mkdir(parents=True, exist_ok=True)

    available_tools = list(tools or [])
    tool_map = {tool.name: tool for tool in available_tools}
    backend = backend or create_backend(model)
    selected_model = str(model or getattr(backend, "model", "") or "")
    run_state = RunState(run_dir=run_path, task=task, model=selected_model)
    ctx = ToolContext(run_dir=run_path, run_state=run_state)

    def log(event: str, detail: str = "") -> None:
        if log_callback:
            log_callback(event, detail)

    messages: list[dict] = [{"role": "user", "content": task}]
    system_prompt = _build_system_prompt(available_tools, extra_instructions)
    api_tools = [tool.to_api_schema() for tool in available_tools]
    reasoning_log_path = run_path / "reasoning_log.jsonl"
    task_start = time.time()
    final_text = ""
    context_memory: list[str] = []
    tool_call_signatures: dict[str, list[int]] = {}
    turn = 0
    completed = False

    _write_jsonl(
        reasoning_log_path,
        {
            "type": "task_start",
            "timestamp": datetime.now().isoformat(),
            "task": task,
            "model": selected_model,
            "tools": [tool.name for tool in available_tools],
        },
    )
    log("start", task)

    while turn < max_turns:
        turn += 1
        turn_start = time.time()
        log("turn", f"{turn}/{max_turns}")

        request_messages = _prepare_messages_for_context(messages, run_state, context_memory)
        try:
            response = backend.create_message(
                system=system_prompt,
                messages=request_messages,
                tools=api_tools,
                max_tokens=8192,
            )
        except Exception as exc:  # noqa: BLE001 - backend boundary
            final_text = f"API error: {exc}"
            _write_jsonl(
                reasoning_log_path,
                {"type": "api_error", "timestamp": datetime.now().isoformat(), "turn": turn, "error": str(exc)},
            )
            break

        messages.append({"role": "assistant", "content": backend.format_assistant_content(response)})
        visible_texts = [text for text in response.text_blocks if not text.startswith("[Thinking] ")]
        if visible_texts:
            final_text = "\n".join(visible_texts)

        run_state.note_assistant_turn(
            turn=turn,
            text="\n".join(visible_texts or response.text_blocks),
            tool_calls=[{"name": call.name, "input": call.input} for call in response.tool_calls],
        )
        _write_jsonl(
            reasoning_log_path,
            {
                "type": "llm_response",
                "timestamp": datetime.now().isoformat(),
                "turn": turn,
                "stop_reason": response.stop_reason,
                "text": "\n".join(visible_texts),
                "tool_calls": [{"name": call.name, "input": call.input} for call in response.tool_calls],
                "usage": {"input_tokens": response.input_tokens, "output_tokens": response.output_tokens},
            },
        )

        if not response.tool_calls:
            completed = True
            break

        all_results: list[list[dict]] = []
        for tool_call in response.tool_calls:
            tool_name = tool_call.name
            tool_input = tool_call.input
            ctx.turn = turn
            ctx.active_tool_name = tool_name
            signature = _tool_call_signature(tool_name, tool_input)
            history = tool_call_signatures.setdefault(signature, [])
            history.append(turn)

            run_state.note_tool_call(turn=turn, tool_name=tool_name, tool_input=tool_input)
            tool = tool_map.get(tool_name)
            tool_start = time.time()
            if tool is None:
                result_content = [{"type": "text", "text": f"Error: Unknown tool '{tool_name}'"}]
            else:
                try:
                    result_content = await _execute_tool(tool, tool_input, ctx)
                except Exception as exc:  # noqa: BLE001 - tool boundary
                    result_content = [{"type": "text", "text": f"Error executing {tool_name}: {exc}"}]

            duration_s = round(time.time() - tool_start, 2)
            summary = _text_summary(result_content, max_len=900)
            run_state.note_tool_result(
                turn=turn,
                tool_name=tool_name,
                tool_input=tool_input,
                result_summary=summary,
                duration_s=duration_s,
            )
            context_memory.append(
                f"- turn {turn} {tool_name}({json.dumps(tool_input, ensure_ascii=False)[:160]}): {summary}"
            )
            context_memory = context_memory[-80:]
            _write_jsonl(
                reasoning_log_path,
                {
                    "type": "tool_result",
                    "timestamp": datetime.now().isoformat(),
                    "turn": turn,
                    "tool": tool_name,
                    "duration_s": duration_s,
                    "result_summary": summary,
                    "repeat_count": len(history),
                },
            )
            all_results.append(result_content)
            ctx.active_tool_name = ""

        messages.append(backend.format_tool_results(response.tool_calls, all_results))
        await asyncio.sleep(0)
        _write_jsonl(
            reasoning_log_path,
            {
                "type": "turn_end",
                "timestamp": datetime.now().isoformat(),
                "turn": turn,
                "duration_s": round(time.time() - turn_start, 2),
            },
        )

    if turn >= max_turns and not completed:
        log("turn", f"final-summary (max_turns={max_turns} reached)")
        messages.append(
            {
                "role": "user",
                "content": (
                    f"You have reached the maximum of {max_turns} tool-using turns. "
                    "Do not call any more tools. Based on the evidence already gathered, "
                    "produce the best possible final answer for the user now in the same "
                    "language as the original task. If information is incomplete, state "
                    "what is known, what is missing, and give your best-effort conclusion."
                ),
            }
        )
        request_messages = _prepare_messages_for_context(messages, run_state, context_memory)
        try:
            response = backend.create_message(
                system=system_prompt,
                messages=request_messages,
                tools=[],
                max_tokens=8192,
            )
            messages.append({"role": "assistant", "content": backend.format_assistant_content(response)})
            visible_texts = [text for text in response.text_blocks if not text.startswith("[Thinking] ")]
            if visible_texts:
                final_text = "\n".join(visible_texts)
            run_state.note_assistant_turn(
                turn=turn + 1,
                text="\n".join(visible_texts or response.text_blocks),
                tool_calls=[],
            )
            _write_jsonl(
                reasoning_log_path,
                {
                    "type": "llm_response",
                    "timestamp": datetime.now().isoformat(),
                    "turn": turn + 1,
                    "stop_reason": response.stop_reason,
                    "text": "\n".join(visible_texts),
                    "tool_calls": [],
                    "usage": {"input_tokens": response.input_tokens, "output_tokens": response.output_tokens},
                    "forced_summary": True,
                },
            )
        except Exception as exc:  # noqa: BLE001 - backend boundary
            suffix = f"Reached max_turns ({max_turns}) and forced-summary call failed: {exc}"
            final_text = f"{final_text}\n\n{suffix}".strip() if final_text else suffix
            _write_jsonl(
                reasoning_log_path,
                {"type": "api_error", "timestamp": datetime.now().isoformat(), "turn": turn + 1, "error": str(exc), "forced_summary": True},
            )

    total_duration = round(time.time() - task_start, 2)
    report_path = run_path / "report.md"
    report_path.write_text(final_text, encoding="utf-8")
    summary = {
        "task": task,
        "model": selected_model,
        "turns": turn,
        "total_duration_s": total_duration,
        "run_dir": str(run_path),
        "run_state_dir": str(run_state.state_dir),
        "reasoning_log_file": "reasoning_log.jsonl",
    }
    (run_path / "agent_log.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")
    _write_jsonl(
        reasoning_log_path,
        {"type": "task_end", "timestamp": datetime.now().isoformat(), "turn": turn, "total_duration_s": total_duration},
    )

    return {
        "result": final_text,
        "turns": turn,
        "run_dir": str(run_path),
        "run_state_dir": str(run_state.state_dir),
        "reasoning_log": str(reasoning_log_path),
        "total_duration_s": total_duration,
    }
