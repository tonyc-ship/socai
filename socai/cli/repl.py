"""Interactive Socai REPL — prompt, slash commands, model picker, event rendering."""

from __future__ import annotations

import asyncio
import sys

from prompt_toolkit import PromptSession
from prompt_toolkit.application import Application
from prompt_toolkit.completion import Completer, Completion
from prompt_toolkit.filters import Condition
from prompt_toolkit.formatted_text import FormattedText
from prompt_toolkit.history import InMemoryHistory
from prompt_toolkit.key_binding import KeyBindings
from prompt_toolkit.layout import HSplit, Layout, Window
from prompt_toolkit.layout.controls import FormattedTextControl
from prompt_toolkit.patch_stdout import patch_stdout
from prompt_toolkit.styles import Style

from socai.agent.backends import (
    PROVIDERS,
    _provider_has_key,
    default_model_for_provider,
    has_any_api_key,
    resolve_model_provider,
    save_api_key,
)
from socai.browser.cdp import BrowserTaskSessionManager
from socai.cli.runner import DEFAULT_MAX_TURNS, run_agent_task


PROVIDER_ORDER = ("kimi", "qwen", "openai", "anthropic")

SLASH_COMMANDS: list[tuple[str, str]] = [
    ("model", "Choose the active LLM model"),
    ("exit", "Exit the CLI"),
]


# ---------- Event rendering ------------------------------------------------


def print_agent_event(event: str, detail: str) -> None:
    """Render an agent event from the runner to stderr."""
    detail = detail or ""
    if event == "start":
        print(f"\n▸ task: {detail}", file=sys.stderr)
    elif event == "turn":
        print(f"\n── turn {detail} ──", file=sys.stderr)
    elif event == "thinking":
        for line in detail.splitlines() or [""]:
            print(f"  · {line}", file=sys.stderr)
    elif event == "assistant_text":
        for line in detail.splitlines() or [""]:
            print(f"  {line}", file=sys.stderr)
    elif event == "tool_call":
        print(f"  → {detail}", file=sys.stderr)
    elif event == "tool_result":
        print(f"  ← {detail}", file=sys.stderr)
    elif event == "tool_error":
        print(f"  ✗ {detail}", file=sys.stderr)
    elif event == "api_error":
        print(f"  ✗ API error: {detail}", file=sys.stderr)
    elif event == "done":
        print(f"\n✓ done · run dir: {detail}", file=sys.stderr)
    else:
        print(f"  [{event}] {detail}", file=sys.stderr)


def print_browser_event(message: str) -> None:
    print(f"[socai] {message}", file=sys.stderr)


def print_agent_result(result: dict) -> None:
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


# ---------- Slash command completion ---------------------------------------


class _SlashCompleter(Completer):
    def get_completions(self, document, complete_event):
        text = document.text
        if not text.startswith("/"):
            return
        first_token = text.split(" ", 1)[0]
        prefix = first_token[1:]
        for name, desc in SLASH_COMMANDS:
            if name.startswith(prefix):
                yield Completion(
                    "/" + name,
                    start_position=-len(first_token),
                    display=name,
                    display_meta=desc,
                )


# ---------- Main prompt key bindings ---------------------------------------


def _build_keybindings() -> KeyBindings:
    kb = KeyBindings()

    @kb.add("enter")
    def _(event):
        buf = event.current_buffer
        if buf.complete_state:
            completion = buf.complete_state.current_completion
            if completion is not None:
                buf.apply_completion(completion)
                return
            buf.cancel_completion()
            return
        if buf.text.strip():
            buf.validate_and_handle()

    @kb.add("escape", "enter")
    def _(event):
        event.current_buffer.insert_text("\n")

    @kb.add("up")
    def _(event):
        buf = event.current_buffer
        if buf.complete_state:
            buf.complete_previous()
            return
        if buf.document.cursor_position_row > 0:
            buf.cursor_up()
        else:
            buf.history_backward()

    @kb.add("down")
    def _(event):
        buf = event.current_buffer
        if buf.complete_state:
            buf.complete_next()
            return
        if buf.document.cursor_position_row < buf.document.line_count - 1:
            buf.cursor_down()
        else:
            buf.history_forward()

    return kb


# ---------- Model picker ---------------------------------------------------


def _model_options() -> list[tuple[str, str, str]]:
    """(label, provider, model) tuples for the picker."""
    options: list[tuple[str, str, str]] = []
    for provider in PROVIDER_ORDER:
        config = PROVIDERS.get(provider)
        if config is None:
            continue
        model = default_model_for_provider(provider)
        options.append((f"{config.display_name} — {model}", provider, model))
    return options


async def _pick_model_inline(current_model: str | None) -> str | None:
    """Inline arrow-key model picker. Returns chosen model id, or None on cancel.

    The rendered region is erased on exit so the menu doesn't pollute scrollback.
    """
    options = _model_options()
    selected = 0
    for i, (_, _, model) in enumerate(options):
        if model == current_model:
            selected = i
            break

    style = Style.from_dict(
        {
            "title": "bold",
            "row.selected": "reverse",
            "row.current": "ansigreen",
            "hint": "italic ansibrightblack",
        }
    )

    def render() -> FormattedText:
        rows: list[tuple[str, str]] = [
            ("class:title", "Select model\n"),
            ("class:hint", "↑/↓ to move, enter to confirm, esc to cancel\n\n"),
        ]
        for i, (label, _, model) in enumerate(options):
            marker = "●" if model == current_model else " "
            line = f" {marker} {label}\n"
            if i == selected:
                rows.append(("class:row.selected", line))
            elif model == current_model:
                rows.append(("class:row.current", line))
            else:
                rows.append(("", line))
        return FormattedText(rows)

    kb = KeyBindings()

    @kb.add("up")
    @kb.add("c-p")
    def _(event):
        nonlocal selected
        selected = (selected - 1) % len(options)
        event.app.invalidate()

    @kb.add("down")
    @kb.add("c-n")
    def _(event):
        nonlocal selected
        selected = (selected + 1) % len(options)
        event.app.invalidate()

    @kb.add("enter")
    def _(event):
        event.app.exit(result=options[selected][2])

    @kb.add("escape", eager=True)
    @kb.add("c-c")
    @kb.add("c-g")
    def _(event):
        event.app.exit(result=None)

    layout = Layout(HSplit([Window(FormattedTextControl(render), always_hide_cursor=True, wrap_lines=True)]))
    app: Application = Application(
        layout=layout,
        key_bindings=kb,
        full_screen=False,
        style=style,
        mouse_support=False,
        erase_when_done=True,
    )
    return await app.run_async()


# ---------- API key prompt (Esc to cancel) ---------------------------------


async def _prompt_api_key(provider_label: str) -> str | None:
    """Single-line key prompt. Enter accepts, Esc/Ctrl-C cancels (returns None)."""
    kb = KeyBindings()

    @kb.add("escape", eager=True)
    def _(event):
        event.app.exit(result=None)

    @kb.add("c-c")
    def _(event):
        event.app.exit(result=None)

    session: PromptSession = PromptSession(key_bindings=kb)
    try:
        with patch_stdout():
            value = await session.prompt_async(f"{provider_label} API key: ")
    except (EOFError, KeyboardInterrupt):
        return None
    if value is None:
        return None
    return value.strip() or None


async def _handle_model_command(state: dict) -> None:
    active = _resolve_active_model(state["model"])
    chosen = await _pick_model_inline(active)
    if chosen is None:
        return  # erase_when_done already cleared the picker

    provider = resolve_model_provider(chosen)
    if not _provider_has_key(provider):
        config = PROVIDERS[provider]
        env_hint = " or ".join(f"${name}" for name in config.api_key_env)
        print(f"[socai] No API key found for {config.display_name}.")
        print(f"        Enter one now (esc to cancel; you can also set {env_hint}).")
        key = await _prompt_api_key(config.display_name)
        if not key:
            print("[socai] model unchanged.")
            return
        try:
            path = save_api_key(provider, key)
        except Exception as exc:  # noqa: BLE001 - user-facing
            print(f"[socai] failed to save key: {exc}")
            return
        print(f"[socai] Saved {config.display_name} key to {path}")

    state["model"] = chosen


# ---------- Helpers --------------------------------------------------------


def _resolve_active_model(model: str | None) -> str:
    if model:
        return model
    provider = resolve_model_provider()
    return default_model_for_provider(provider)


# ---------- REPL -----------------------------------------------------------


async def repl() -> int:
    _ensure_llm_key()

    state: dict = {"model": None}
    history = InMemoryHistory()
    completer = _SlashCompleter()
    keybindings = _build_keybindings()

    @Condition
    def _slash_active() -> bool:
        from prompt_toolkit.application.current import get_app

        try:
            return get_app().current_buffer.text.startswith("/")
        except Exception:  # noqa: BLE001
            return False

    session: PromptSession = PromptSession(
        history=history,
        completer=completer,
        complete_while_typing=_slash_active,
        multiline=True,
        key_bindings=keybindings,
        mouse_support=False,
    )

    manager = BrowserTaskSessionManager(on_event=print_browser_event)

    def print_header() -> None:
        print(f"Current model: {_resolve_active_model(state['model'])}")
        print(
            "Enter your task description. Type / for commands. "
            "Alt+Enter for newline. Ctrl+C to exit."
        )

    print_header()

    try:
        while True:
            try:
                with patch_stdout():
                    line = await session.prompt_async("socai> ")
            except EOFError:
                print()
                break
            except KeyboardInterrupt:
                print()
                break

            line = (line or "").strip()
            if not line:
                continue

            if line.startswith("/"):
                cmd, _, _rest = line[1:].partition(" ")
                cmd = cmd.lower()
                if cmd == "model":
                    await _handle_model_command(state)
                    print_header()
                elif cmd == "exit":
                    break
                else:
                    print(f"[socai] unknown command: /{cmd}", file=sys.stderr)
                continue

            try:
                result = await run_agent_task(
                    manager,
                    line,
                    model=state["model"],
                    max_turns=DEFAULT_MAX_TURNS,
                    on_agent_event=print_agent_event,
                )
            except Exception as exc:  # noqa: BLE001 - interactive diagnostic
                print(f"[socai] error: {exc}", file=sys.stderr)
                continue

            print_agent_result(result)
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


def main(argv: list[str] | None = None) -> int:
    try:
        return asyncio.run(repl())
    except KeyboardInterrupt:
        print()
        return 130
    except Exception as exc:  # noqa: BLE001 - command-line diagnostic
        print(f"[socai] error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
