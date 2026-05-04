"""Minimal Socai command-line interface."""

from __future__ import annotations

import argparse
import asyncio
import sys

from socai.agent.backends import PROVIDERS, SOCAI_AUTH_FILE, has_any_api_key, save_api_key
from socai.agent.loop import run_agent
from socai.agent.run_logging import JsonlEventLogger, current_traceback, make_run_dir
from socai.browser.cdp import BrowserTaskSessionManager
from socai.browser.tools.browser import build_browser_tools
from socai.sites.xhs import XhsRuntime
from socai.sites.xhs.runtime import XHS_HOME_URL
from socai.sites.xhs.tools import build_xhs_tools


EXIT_COMMANDS = {"exit", "quit", "q", ":q"}
PROVIDER_ORDER = ("kimi", "qwen", "openai", "anthropic")
DEFAULT_START_URL = "about:blank"

XHS_KEYWORD_HINTS = ("小红书", "xiaohongshu", "xhs")

_AGENT_INSTRUCTIONS = """\
You are running inside the Socai CLI. A fresh browser tab has been opened over a
reused CDP connection — use the browser tools to drive it.

Tool selection rules:
- If the task is about Xiaohongshu (小红书 / xhs / xiaohongshu.com), prefer the
  `xhs_*` site tools (search, read, close). They handle anti-bot quirks.
- For any other site, use the generic `browser_*` tools. Start by navigating to
  the right URL with `browser_navigate`. Use `browser_click_selector`,
  `browser_fill`, and `browser_wait_for_selector` instead of blind coordinate
  clicks whenever you can identify a CSS selector.
- Use `browser_screenshot` only when DOM-based extraction is insufficient.

Reply in the same language as the task. Ground every claim in tool output, and
mention the saved artifact path only when it adds value.
"""


def _looks_like_xhs_task(text: str) -> bool:
    lowered = text.lower()
    return any(hint in lowered for hint in XHS_KEYWORD_HINTS)


async def _run_agent_task(
    manager: BrowserTaskSessionManager,
    task_text: str,
    *,
    model: str | None,
    max_turns: int,
) -> dict:
    task_text = str(task_text or "").strip()
    if not task_text:
        raise ValueError("Task is empty.")

    is_xhs = _looks_like_xhs_task(task_text)
    start_url = XHS_HOME_URL if is_xhs else DEFAULT_START_URL
    site = "xiaohongshu" if is_xhs else ""
    run_dir = make_run_dir(task_text)
    cli_log_path = run_dir / "cli_events.jsonl"
    cli_log = JsonlEventLogger(cli_log_path)

    reused = manager.browser is not None
    previous_on_event = manager.on_event

    def emit_browser_event(message: str) -> None:
        cli_log.write("browser_event", message=message)
        if previous_on_event:
            previous_on_event(message)

    def emit_agent_event(event: str, detail: str = "") -> None:
        cli_log.write("agent_event", event=event, detail=detail)
        print(f"[agent] {event}: {detail}", file=sys.stderr)

    manager.on_event = emit_browser_event
    task = None
    try:
        cli_log.write(
            "cli_task_start",
            task=task_text,
            run_dir=str(run_dir),
            start_url=start_url,
            site=site or "generic",
            connection="reused" if reused else "new",
        )
        task = await manager.create_task(start_url=start_url, label=task_text[:80], site=site)
        cli_log.write("browser_task_created", task=task.to_dict())
        browser = await manager.ensure_browser()

        tools = list(build_browser_tools(browser))
        # XHS site tools are always exposed so the agent can switch to them mid-task,
        # but they are gated behind the "you are on xiaohongshu.com" check inside the
        # runtime — they will refuse to run on the wrong page.
        runtime = XhsRuntime(task.page)
        tools.extend(build_xhs_tools(runtime))

        result = await run_agent(
            task_text,
            tools=tools,
            run_dir=run_dir,
            max_turns=max_turns,
            model=model,
            extra_instructions=_AGENT_INSTRUCTIONS,
            log_callback=emit_agent_event,
        )
        result.update(
            {
                "connection": "reused" if reused else "new",
                "browser_task_id": task.task_id,
                "start_url": start_url,
                "site": site or "generic",
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
    except Exception as exc:  # noqa: BLE001 - persistent CLI diagnostics
        cli_log.write(
            "cli_task_error",
            error=str(exc),
            traceback=current_traceback(),
        )
        raise
    finally:
        manager.on_event = previous_on_event
        # Always close the per-task tab so the user's browser doesn't accumulate
        # one tab per CLI prompt. The browser CDP connection itself stays open
        # (BrowserTaskSessionManager owns it across tasks).
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
                print(f"[socai] warning: failed to close task tab: {exc}", file=sys.stderr)


def _print_agent_result(result: dict) -> None:
    print()
    print(str(result.get("result") or "").strip())
    print()
    print(
        "[socai] "
        f"connection={result.get('connection')} "
        f"site={result.get('site')} "
        f"task_id={result.get('browser_task_id')} "
        f"turns={result.get('turns')} "
        f"run_dir={result.get('run_dir')} "
        f"cli_log={result.get('cli_log')}"
    )


async def repl(args: argparse.Namespace) -> int:
    _ensure_llm_key()
    manager = BrowserTaskSessionManager(
        on_event=lambda message: print(f"[socai] {message}", file=sys.stderr),
    )
    print("Socai CLI. Type a task, or `q` to exit.")
    try:
        while True:
            try:
                line = input("socai> ").strip()
            except EOFError:
                print()
                break
            except KeyboardInterrupt:
                print()
                break

            if not line:
                continue
            if line.lower() in EXIT_COMMANDS:
                break

            try:
                result = await _run_agent_task(
                    manager,
                    line,
                    model=args.model,
                    max_turns=args.max_turns,
                )
            except Exception as exc:  # noqa: BLE001 - interactive diagnostic
                print(f"[socai] error: {exc}", file=sys.stderr)
                continue

            _print_agent_result(result)
        return 0
    finally:
        await manager.shutdown()


def _ensure_llm_key() -> None:
    if has_any_api_key():
        return
    if not sys.stdin.isatty():
        raise RuntimeError(
            "No LLM API key found. Run `uv run socai` in an interactive terminal to set one, "
            "or set OPENAI_API_KEY / ANTHROPIC_API_KEY / KIMI_API_KEY / QWEN_API_KEY."
        )

    print("No LLM API key found. Set one for Socai.")
    for index, provider in enumerate(PROVIDER_ORDER, start=1):
        config = PROVIDERS[provider]
        print(f"{index}. {config.display_name} ({config.api_key_env[0]})")

    selected_provider = ""
    while selected_provider not in PROVIDERS:
        raw = input("Provider [1]: ").strip()
        if not raw:
            selected_provider = PROVIDER_ORDER[0]
            break
        if raw.isdigit() and 1 <= int(raw) <= len(PROVIDER_ORDER):
            selected_provider = PROVIDER_ORDER[int(raw) - 1]
            break
        lowered = raw.lower()
        if lowered in PROVIDERS:
            selected_provider = lowered
            break
        print("Unknown provider.")

    config = PROVIDERS[selected_provider]
    key = ""
    while not key:
        key = input(f"{config.display_name} API key: ").strip()
        if not key:
            print("API key cannot be empty.")

    path = save_api_key(selected_provider, key)
    print(f"[socai] Saved {config.display_name} key to {path}")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Socai CLI")
    parser.add_argument("--model", default=None, help="Override the LLM model id.")
    parser.add_argument("--max-turns", type=int, default=12, help="Max agent turns per task (default 12).")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return asyncio.run(repl(args))
    except KeyboardInterrupt:
        print()
        return 130
    except Exception as exc:  # noqa: BLE001 - command-line diagnostic
        print(f"[socai] error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
