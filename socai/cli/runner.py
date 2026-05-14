"""Headless task runner — wires the browser session manager to the agent loop.

This module contains no UI. Callers (CLI, Tauri app, tests) supply a
``BrowserTaskSessionManager`` and an optional event callback; the runner
returns a structured result dict.
"""

from __future__ import annotations

from typing import Callable

from socai.agent.backends import create_backend
from socai.agent.loop import run_agent
from socai.agent.run_logging import JsonlEventLogger, current_traceback, make_run_dir
from socai.browser.cdp import BrowserTaskSessionManager
from socai.browser.tools.browser import build_browser_tools
from socai.media import MediaProcessor
from socai.sites.toolbox import SiteToolboxTool
from socai.sites.xhs import XhsRuntime
from socai.sites.xhs.tools import build_xhs_tools


XHS_SITE = "xiaohongshu"
DEFAULT_START_URL = "https://www.xiaohongshu.com"
DEFAULT_MAX_TURNS = 30

AGENT_INSTRUCTIONS = """\
You are running inside the Socai CLI. The browser is locked to Xiaohongshu
(xiaohongshu.com / 小红书) — every task is an XHS task. Do NOT navigate to or
search on any other website. If the user's task cannot be answered from XHS,
say so plainly and stop.

A fresh tab has been opened on xiaohongshu.com over a reused CDP connection.

How to work:
- On your first turn, call `site_toolbox` with `site="xiaohongshu"` to unlock
  the `xhs_*` site tools.
- After unlocking, prefer the `xhs_*` site tools (search_notes, topic_scan,
  read_note, …) over generic `browser_*` tools — they encapsulate the site's
  quirks.
- Use generic `browser_*` tools only when no `xhs_*` tool fits (e.g. clicking
  a non-standard UI control on an XHS page).
- Use `browser_screenshot` early when visual state matters; verify modal
  changes and navigation with a fresh screenshot or `xhs_page_state`.

Reply in the same language as the task. Ground every claim in tool output,
and mention the saved artifact path only when it adds value.
"""


AgentEventCallback = Callable[[str, str], None]
BrowserEventCallback = Callable[[str], None]


async def run_agent_task(
    manager: BrowserTaskSessionManager,
    task_text: str,
    *,
    model: str | None = None,
    max_turns: int = DEFAULT_MAX_TURNS,
    on_agent_event: AgentEventCallback | None = None,
    on_browser_event: BrowserEventCallback | None = None,
) -> dict:
    """Run a single agent task end-to-end.

    Creates a fresh browser tab, runs the agent loop with site + browser tools,
    persists CLI events alongside the agent run, and always closes the tab on
    exit. The caller-supplied callbacks receive structured events for UI
    rendering — runtime never prints.
    """
    task_text = str(task_text or "").strip()
    if not task_text:
        raise ValueError("Task is empty.")

    run_dir = make_run_dir(task_text)
    cli_log_path = run_dir / "cli_events.jsonl"
    cli_log = JsonlEventLogger(cli_log_path)
    backend = None
    start_url = DEFAULT_START_URL  # always xhs — TUI is locked to xiaohongshu

    reused = manager.browser is not None
    previous_on_event = manager.on_event

    def emit_browser_event(message: str) -> None:
        cli_log.write("browser_event", message=message)
        if previous_on_event:
            previous_on_event(message)
        if on_browser_event:
            on_browser_event(message)

    def emit_agent_event(event: str, detail: str = "") -> None:
        cli_log.write("agent_event", event=event, detail=detail)
        if on_agent_event:
            on_agent_event(event, detail)

    manager.on_event = emit_browser_event
    task = None
    try:
        backend = create_backend(model)
        cli_log.write(
            "cli_task_start",
            task=task_text,
            run_dir=str(run_dir),
            start_url=start_url,
            connection="reused" if reused else "new",
        )
        task = await manager.create_task(
            start_url=start_url,
            label=task_text[:80],
            site=XHS_SITE,
            wait_for_load=False,
        )
        cli_log.write("browser_task_created", task=task.to_dict())
        browser = await manager.ensure_browser()

        tools = list(build_browser_tools(browser))
        tools.append(SiteToolboxTool())
        runtime = XhsRuntime(task.page, media=MediaProcessor.for_run_dir(run_dir, backend=backend))
        tools.extend(build_xhs_tools(runtime))

        result = await run_agent(
            task_text,
            backend=backend,
            tools=tools,
            run_dir=run_dir,
            max_turns=max_turns,
            model=model,
            extra_instructions=AGENT_INSTRUCTIONS,
            log_callback=emit_agent_event,
        )
        result.update(
            {
                "connection": "reused" if reused else "new",
                "browser_task_id": task.task_id,
                "start_url": start_url,
                "cli_log": str(cli_log_path),
            }
        )
        cli_log.write(
            "cli_task_result",
            ok=True,
            result={
                "turns": result.get("turns"),
                "total_duration_s": result.get("total_duration_s"),
                "run_dir": result.get("run_dir"),
                "reasoning_log": result.get("reasoning_log"),
                "conversation": result.get("conversation"),
                "report": str(run_dir / "report.md"),
            },
            final_text=str(result.get("result") or ""),
        )
        return result
    except Exception as exc:  # noqa: BLE001 - persistent diagnostics
        cli_log.write(
            "cli_task_error",
            error=str(exc),
            traceback=current_traceback(),
        )
        raise
    finally:
        manager.on_event = previous_on_event
        if task is not None:
            try:
                closed = await manager.close_task(task.task_id)
                cli_log.write("browser_task_closed", task_id=task.task_id, closed=closed)
            except Exception as exc:  # noqa: BLE001 - cleanup best-effort
                cli_log.write(
                    "browser_task_close_error",
                    task_id=task.task_id,
                    error=str(exc),
                    traceback=current_traceback(),
                )
                if on_browser_event:
                    on_browser_event(f"warning: failed to close task tab: {exc}")
