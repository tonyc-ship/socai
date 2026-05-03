"""Minimal Socai command-line interface."""

from __future__ import annotations

import argparse
import asyncio
import sys
import termios
import tty

from socai.agent.backends import PROVIDERS, SOCAI_AUTH_FILE, has_any_api_key, save_api_key
from socai.agent.loop import run_agent
from socai.browser.cdp import BrowserTaskSessionManager
from socai.browser.tools.browser import build_browser_tools
from socai.sites.xhs import XhsRuntime
from socai.sites.xhs.runtime import XHS_HOME_URL
from socai.sites.xhs.tools import build_xhs_tools


EXIT_COMMANDS = {"exit", "quit", "q", ":q", "\x1b"}
PROVIDER_ORDER = ("kimi", "qwen", "openai", "anthropic")

_AGENT_INSTRUCTIONS = """\
You are running inside the Socai CLI.
For each user task, Socai has already opened a fresh Xiaohongshu tab over a
reused CDP browser connection. Use the Xiaohongshu tools to search/read notes
when the task is about Xiaohongshu. Use generic browser tools only when needed
for verification or recovery. Finish with a concise Chinese answer grounded in
tool results, and mention the artifact/run output only when useful.
"""


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

    reused = manager.browser is not None
    task = await manager.create_task(start_url=XHS_HOME_URL, label=task_text[:80], site="xiaohongshu")
    browser = await manager.ensure_browser()
    runtime = XhsRuntime(task.page)
    tools = [*build_xhs_tools(runtime), *build_browser_tools(browser)]

    result = await run_agent(
        task_text,
        tools=tools,
        max_turns=max_turns,
        model=model,
        extra_instructions=_AGENT_INSTRUCTIONS,
        log_callback=lambda event, detail: print(f"[agent] {event}: {detail}", file=sys.stderr),
    )
    result.update(
        {
            "connection": "reused" if reused else "new",
            "browser_task_id": task.task_id,
        }
    )
    return result


def _print_agent_result(result: dict) -> None:
    print()
    print(str(result.get("result") or "").strip())
    print()
    print(
        "[socai] "
        f"connection={result.get('connection')} "
        f"task_id={result.get('browser_task_id')} "
        f"turns={result.get('turns')} "
        f"run_dir={result.get('run_dir')}"
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
                line = _read_command("socai> ").strip()
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
                    model=None,
                    max_turns=12,
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


def _read_command(prompt: str) -> str:
    if not sys.stdin.isatty():
        return input(prompt)

    fd = sys.stdin.fileno()
    old_settings = termios.tcgetattr(fd)
    chars: list[str] = []
    sys.stdout.write(prompt)
    sys.stdout.flush()
    try:
        tty.setcbreak(fd)
        while True:
            ch = sys.stdin.read(1)
            if ch in {"\n", "\r"}:
                print()
                return "".join(chars)
            if ch == "\x03":
                raise KeyboardInterrupt
            if ch == "\x04":
                raise EOFError
            if ch == "\x1b":
                print()
                return "\x1b"
            if ch in {"\x7f", "\b"}:
                if chars:
                    chars.pop()
                    sys.stdout.write("\b \b")
                    sys.stdout.flush()
                continue
            if ch >= " ":
                chars.append(ch)
                sys.stdout.write(ch)
                sys.stdout.flush()
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, old_settings)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Socai CLI")
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
