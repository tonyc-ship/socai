"""Socai tool daemon — long-lived process owning one browser tool tab.

Architecture:
- Single ``BrowserTaskSessionManager`` (reuses the user's logged-in Chrome via
  CDP, same path as the REPL).
- One persistent "tool tab" (``BrowserTaskSession``) created lazily on first
  tool call; recreated if the user closed it.
- One cached ``XhsRuntime`` paired with the tool tab; rebuilt only when the
  tab is rebuilt.
- Unix-domain socket at ``~/.socai/daemon.sock``; JSON-per-line protocol.
- All tool calls serialized by a single asyncio.Lock — the browser tab can't
  drive two things at once.
- Auto-shuts down after ``IDLE_TIMEOUT_S`` of no activity.
"""

from __future__ import annotations

import asyncio
import json
import os
import signal
import sys
import time
import traceback
from pathlib import Path
from typing import Any

from socai.agent.run_logging import make_run_dir
from socai.agent.tool import ToolContext
from socai.browser.cdp import BrowserTaskSessionManager
from socai.browser.cdp.task_session import BrowserTaskSession
from socai.media import MediaProcessor
from socai.sites.xhs import XhsRuntime
from socai.sites.xhs.tools import (
    XhsReadNoteTool,
    XhsSearchNotesTool,
    XhsTopicScanTool,
)


SOCAI_HOME = Path(os.environ.get("SOCAI_HOME", str(Path.home() / ".socai")))
SOCKET_PATH = SOCAI_HOME / "daemon.sock"
PID_PATH = SOCAI_HOME / "daemon.pid"
LOG_PATH = SOCAI_HOME / "daemon.log"

TOOL_TAB_LABEL = "socai-tools"
# Chrome navigates the new tab to this URL the moment it appears (fast path
# in BrowserSession.new_page). search_notes / topic_scan do their own state
# polling, so we don't need the slow two-phase create-then-navigate dance.
TOOL_TAB_START_URL = "https://www.xiaohongshu.com"

IDLE_TIMEOUT_S = 3 * 60 * 60  # 3 hours of no requests → auto-shutdown
IDLE_CHECK_INTERVAL_S = 60


def _log(msg: str) -> None:
    """Append a timestamped line to the daemon log."""
    try:
        SOCAI_HOME.mkdir(parents=True, exist_ok=True)
        with LOG_PATH.open("a", encoding="utf-8") as fh:
            fh.write(msg.rstrip() + "\n")
    except Exception:
        pass


class DaemonState:
    """Mutable runtime state held by the daemon process."""

    def __init__(self) -> None:
        self.manager = BrowserTaskSessionManager(on_event=lambda m: _log(f"[browser] {m}"))
        self.tool_tab: BrowserTaskSession | None = None
        self.runtime: XhsRuntime | None = None
        self.lock = asyncio.Lock()  # serialize all tool calls
        self.started_at = time.monotonic()
        self.last_activity = time.monotonic()

    def touch(self) -> None:
        """Mark activity so the idle-shutdown watchdog resets."""
        self.last_activity = time.monotonic()

    async def ensure_tool_tab(self) -> tuple[BrowserTaskSession, XhsRuntime]:
        """Return (tab, runtime) for the persistent tool tab.

        Recreates either if the user closed the tab in Chrome.
        """
        browser = await self.manager.ensure_browser()
        existing = self.tool_tab
        if existing is not None:
            try:
                targets = await browser.list_pages()
                if any(t.target_id == existing.target_id for t in targets):
                    assert self.runtime is not None  # paired with tool_tab
                    return existing, self.runtime
            except Exception as exc:  # noqa: BLE001 - fall through to recreate
                _log(f"[tab] list_pages failed, recreating: {exc}")
            _log("[tab] previous tool tab is gone, recreating")
            self.tool_tab = None
            self.runtime = None

        task = await self.manager.create_task(
            start_url=TOOL_TAB_START_URL,
            label=TOOL_TAB_LABEL,
            site="xiaohongshu",
            wait_for_load=False,
        )
        runtime = XhsRuntime(task.page)
        self.tool_tab = task
        self.runtime = runtime
        _log(f"[tab] created tool tab task_id={task.task_id} target={task.target_id}")
        return task, runtime

    async def shutdown(self) -> None:
        # Close the tool tab in the user's Chrome first; manager.shutdown()
        # only stops the CDP connection and would otherwise leave the tab
        # hanging in the browser.
        tab = self.tool_tab
        if tab is not None:
            try:
                await self.manager.close_task(tab.task_id)
                _log(f"[shutdown] closed tool tab {tab.task_id}")
            except Exception as exc:  # noqa: BLE001
                _log(f"[shutdown] close_task error: {exc}")
        try:
            await self.manager.shutdown()
        except Exception as exc:  # noqa: BLE001
            _log(f"[shutdown] manager shutdown error: {exc}")
        self.tool_tab = None
        self.runtime = None


def _build_tool_context(run_dir: Path) -> ToolContext:
    """Build a lightweight ToolContext suitable for a single CLI invocation."""
    run_dir.mkdir(parents=True, exist_ok=True)
    ctx = ToolContext(run_dir=run_dir)
    ctx.enabled_sites = {"xiaohongshu"}
    return ctx


def _resolve_artifact_payload(run_dir: Path, reply: str) -> dict[str, Any]:
    """Read the full artifact JSON the tool persisted. Falls back to reply."""
    try:
        envelope = json.loads(reply)
    except Exception:
        return {"raw_reply": reply}
    if not isinstance(envelope, dict):
        return {"raw_reply": reply}
    artifact_rel = str(envelope.get("artifact") or "")
    if artifact_rel:
        artifact_path = run_dir / artifact_rel
        try:
            return json.loads(artifact_path.read_text(encoding="utf-8"))
        except Exception as exc:  # noqa: BLE001 - degrade gracefully
            _log(f"[artifact] failed to read {artifact_path}: {exc}")
    return envelope


async def _run_search_notes(state: DaemonState, args: dict) -> dict:
    query = str(args.get("query") or "").strip()
    if not query:
        raise ValueError("'query' is required")
    _, runtime = await state.ensure_tool_tab()
    run_dir = make_run_dir(f"cli_search_{query}")
    ctx = _build_tool_context(run_dir)
    runtime.media = None  # search doesn't need media
    tool = XhsSearchNotesTool(runtime)
    reply = await tool.execute({"query": query}, ctx)
    payload = _resolve_artifact_payload(run_dir, reply)
    return {"command": "search_notes", "run_dir": str(run_dir), "data": payload}


async def _run_topic_scan(state: DaemonState, args: dict) -> dict:
    query = str(args.get("query") or "").strip()
    if not query:
        raise ValueError("'query' is required")
    depth = str(args.get("depth") or "standard").strip().lower()
    tab_label = str(args.get("tab_label") or "").strip()

    _, runtime = await state.ensure_tool_tab()
    run_dir = make_run_dir(f"cli_topic_scan_{query}")
    ctx = _build_tool_context(run_dir)
    # Swap in a fresh per-run MediaProcessor. ``deep`` depth uses vision/audio
    # which requires a backend; v1 CLI passes backend=None — that work no-ops
    # gracefully, but agents that need deep media should call the REPL.
    runtime.media = MediaProcessor.for_run_dir(run_dir, backend=None)
    tool = XhsTopicScanTool(runtime)
    params: dict[str, Any] = {"query": query, "depth": depth}
    if tab_label:
        params["tab_label"] = tab_label
    reply = await tool.execute(params, ctx)
    payload = _resolve_artifact_payload(run_dir, reply)
    return {"command": "topic_scan", "run_dir": str(run_dir), "data": payload}


async def _run_extract_note(state: DaemonState, args: dict) -> dict:
    """Open a note by id from the **current** tool-tab page.

    extract_note is a continuation command: the tool tab must already be on a
    waterfall (search/topic_scan result, profile page, etc.) where the target
    note's card is reachable. Without that prior context the underlying
    ``read_note`` will not find the card and will return an error.
    """
    note_id = str(args.get("note_id") or "").strip()
    if not note_id:
        raise ValueError("'note_id' is required")
    level = str(args.get("level") or "lite").strip().lower()
    include_media = bool(args.get("include_media", False))

    _, runtime = await state.ensure_tool_tab()
    run_dir = make_run_dir(f"cli_extract_note_{note_id}")
    ctx = _build_tool_context(run_dir)
    runtime.media = MediaProcessor.for_run_dir(run_dir, backend=None) if include_media else None

    # If a previous extract_note left a note modal open, close it first so the
    # tab is back on the waterfall and the target card is clickable again.
    try:
        page_state = await runtime.detect_state()
    except Exception as exc:  # noqa: BLE001 - tolerate detect failures
        _log(f"[extract_note] detect_state failed, skipping pre-close: {exc}")
        page_state = {}
    # NB: pageState() JS returns the page-kind under the key ``state``
    # (homepage / search_results / note_detail / profile_page).
    if str(page_state.get("state") or "") == "note_detail":
        _log("[extract_note] closing previously-open modal before next extract")
        try:
            await runtime.close_note()
        except Exception as exc:  # noqa: BLE001 - close best-effort
            _log(f"[extract_note] close_note failed: {exc}")

    read_tool = XhsReadNoteTool(runtime)
    reply = await read_tool.execute(
        {"note_id": note_id, "level": level, "include_media": include_media},
        ctx,
    )
    payload = _resolve_artifact_payload(run_dir, reply)
    return {"command": "extract_note", "run_dir": str(run_dir), "data": payload}


COMMAND_HANDLERS = {
    "search_notes": _run_search_notes,
    "topic_scan": _run_topic_scan,
    "extract_note": _run_extract_note,
}


async def _handle_request(state: DaemonState, request: dict) -> dict:
    cmd = str(request.get("cmd") or "")
    req_id = request.get("id")
    args = request.get("args") or {}

    state.touch()

    if cmd == "ping":
        return {"id": req_id, "ok": True, "result": {"pong": True}}
    if cmd == "status":
        tab = state.tool_tab
        now = time.monotonic()
        return {
            "id": req_id,
            "ok": True,
            "result": {
                "pid": os.getpid(),
                "socket": str(SOCKET_PATH),
                "tool_tab": tab.to_dict() if tab else None,
                "uptime_s": round(now - state.started_at, 1),
                "idle_s": round(now - state.last_activity, 1),
                "idle_timeout_s": IDLE_TIMEOUT_S,
            },
        }
    if cmd == "shutdown":
        return {"id": req_id, "ok": True, "result": {"shutting_down": True}, "_shutdown": True}
    if cmd == "reset_tab":
        async with state.lock:
            if state.tool_tab is not None:
                try:
                    await state.manager.close_task(state.tool_tab.task_id)
                except Exception as exc:  # noqa: BLE001
                    _log(f"[reset_tab] close error: {exc}")
                state.tool_tab = None
        return {"id": req_id, "ok": True, "result": {"reset": True}}

    handler = COMMAND_HANDLERS.get(cmd)
    if handler is None:
        return {"id": req_id, "ok": False, "error": f"unknown command: {cmd}"}

    try:
        async with state.lock:
            result = await handler(state, args)
        return {"id": req_id, "ok": True, "result": result}
    except Exception as exc:  # noqa: BLE001 - return error to client
        tb = traceback.format_exc()
        _log(f"[error] {cmd}: {exc}\n{tb}")
        return {"id": req_id, "ok": False, "error": str(exc), "traceback": tb}


async def _serve_client(
    state: DaemonState,
    reader: asyncio.StreamReader,
    writer: asyncio.StreamWriter,
    stop_event: asyncio.Event,
) -> None:
    try:
        while True:
            line = await reader.readline()
            if not line:
                return
            try:
                request = json.loads(line.decode("utf-8"))
            except json.JSONDecodeError as exc:
                err = {"ok": False, "error": f"bad json: {exc}"}
                writer.write((json.dumps(err) + "\n").encode("utf-8"))
                await writer.drain()
                continue

            response = await _handle_request(state, request)
            shutdown_after = response.pop("_shutdown", False)
            writer.write((json.dumps(response, ensure_ascii=False) + "\n").encode("utf-8"))
            await writer.drain()
            if shutdown_after:
                stop_event.set()
                return
    except (asyncio.CancelledError, ConnectionResetError):
        pass
    except Exception as exc:  # noqa: BLE001
        _log(f"[client] handler error: {exc}\n{traceback.format_exc()}")
    finally:
        try:
            writer.close()
            await writer.wait_closed()
        except Exception:
            pass


async def _run_daemon() -> int:
    SOCAI_HOME.mkdir(parents=True, exist_ok=True)
    if SOCKET_PATH.exists():
        try:
            SOCKET_PATH.unlink()
        except Exception as exc:  # noqa: BLE001
            _log(f"[startup] cannot remove stale socket: {exc}")
            return 1

    state = DaemonState()
    stop_event = asyncio.Event()

    async def client_cb(r: asyncio.StreamReader, w: asyncio.StreamWriter) -> None:
        await _serve_client(state, r, w, stop_event)

    server = await asyncio.start_unix_server(client_cb, path=str(SOCKET_PATH))
    os.chmod(SOCKET_PATH, 0o600)
    PID_PATH.write_text(str(os.getpid()), encoding="utf-8")
    _log(f"[startup] listening on {SOCKET_PATH} pid={os.getpid()}")

    loop = asyncio.get_running_loop()
    for sig in (signal.SIGINT, signal.SIGTERM):
        try:
            loop.add_signal_handler(sig, stop_event.set)
        except NotImplementedError:
            pass

    async def idle_watchdog() -> None:
        while not stop_event.is_set():
            try:
                await asyncio.wait_for(stop_event.wait(), timeout=IDLE_CHECK_INTERVAL_S)
                return
            except asyncio.TimeoutError:
                pass
            idle = time.monotonic() - state.last_activity
            if idle >= IDLE_TIMEOUT_S:
                _log(f"[idle] no activity for {idle:.0f}s (>= {IDLE_TIMEOUT_S}s), shutting down")
                stop_event.set()
                return

    try:
        async with server:
            watchdog = asyncio.create_task(idle_watchdog())
            try:
                await stop_event.wait()
            finally:
                watchdog.cancel()
    finally:
        _log("[shutdown] stopping")
        await state.shutdown()
        try:
            if SOCKET_PATH.exists():
                SOCKET_PATH.unlink()
        except Exception:
            pass
        try:
            if PID_PATH.exists():
                PID_PATH.unlink()
        except Exception:
            pass
    return 0


def main(argv: list[str] | None = None) -> int:
    try:
        return asyncio.run(_run_daemon())
    except KeyboardInterrupt:
        return 130


if __name__ == "__main__":
    sys.exit(main())
