"""Argparse-based dispatcher for the ``socai`` command.

No subcommand → interactive REPL (existing behaviour, backwards compatible).
Subcommands → daemon-backed tool calls for external agents (Claude Code,
Codex, MCP clients, scripts). The daemon is auto-started on the first tool
call and auto-shuts-down after 3 hours idle; ``socai stop`` stops it early.

Tool subcommands:
- ``socai search_notes <query>``
- ``socai topic_scan <query> [--depth quick|standard|deep] [--tab 全部|图文|视频|用户]``
- ``socai extract_note --note-id <id> [--level card|lite|deep] [--include-media]``
  (continuation — requires the daemon to already be on a waterfall page)

Lifecycle:
- ``socai stop`` — shut the daemon down.
"""

from __future__ import annotations

import argparse
import json
import sys


def _print_json(payload: object, *, pretty: bool) -> None:
    if pretty:
        sys.stdout.write(json.dumps(payload, ensure_ascii=False, indent=2) + "\n")
    else:
        sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")


def _emit_meta(result: dict) -> None:
    """Print non-data daemon info (run_dir, etc.) to stderr."""
    run_dir = result.get("run_dir")
    if run_dir:
        sys.stderr.write(f"[socai] run_dir={run_dir}\n")


def _cmd_search_notes(args: argparse.Namespace) -> int:
    from socai.cli.daemon_client import DaemonError, send

    try:
        result = send("search_notes", {"query": args.query})
    except DaemonError as exc:
        sys.stderr.write(f"[socai] error: {exc}\n")
        return 1
    _emit_meta(result)
    _print_json(result.get("data") or {}, pretty=args.pretty)
    return 0


def _cmd_topic_scan(args: argparse.Namespace) -> int:
    from socai.cli.daemon_client import DaemonError, send

    payload = {"query": args.query, "depth": args.depth}
    if args.tab:
        payload["tab_label"] = args.tab
    try:
        result = send("topic_scan", payload, request_timeout=900.0)
    except DaemonError as exc:
        sys.stderr.write(f"[socai] error: {exc}\n")
        return 1
    _emit_meta(result)
    _print_json(result.get("data") or {}, pretty=args.pretty)
    return 0


def _cmd_extract_note(args: argparse.Namespace) -> int:
    from socai.cli.daemon_client import DaemonError, send

    payload = {
        "note_id": args.note_id,
        "level": args.level,
        "include_media": args.include_media,
    }
    try:
        result = send("extract_note", payload, request_timeout=600.0)
    except DaemonError as exc:
        sys.stderr.write(f"[socai] error: {exc}\n")
        return 1
    _emit_meta(result)
    _print_json(result.get("data") or {}, pretty=args.pretty)
    return 0


def _cmd_stop(_args: argparse.Namespace) -> int:
    from socai.cli.daemon_client import stop_daemon

    stopped = stop_daemon()
    sys.stderr.write("[socai] daemon stopped\n" if stopped else "[socai] daemon was not running\n")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="socai",
        description=(
            "socai — XHS-savvy browser agent. Run with no arguments for the "
            "interactive REPL, or use a subcommand to call a single tool "
            "(useful from Claude Code / Codex / scripts)."
        ),
    )
    sub = parser.add_subparsers(dest="command")

    common_pretty = argparse.ArgumentParser(add_help=False)
    common_pretty.add_argument(
        "--pretty",
        action="store_true",
        help="Pretty-print the JSON result with indentation.",
    )

    p = sub.add_parser("search_notes", parents=[common_pretty], help="Search XHS and return note card previews.")
    p.add_argument("query", help="Search keyword.")
    p.set_defaults(func=_cmd_search_notes)

    p = sub.add_parser("topic_scan", parents=[common_pretty], help="Search + sample top notes + return one bundle.")
    p.add_argument("query", help="Topic keyword.")
    p.add_argument("--depth", choices=["quick", "standard", "deep"], default="standard")
    p.add_argument("--tab", choices=["全部", "图文", "视频", "用户"], default="", help="Optional search-result tab filter.")
    p.set_defaults(func=_cmd_topic_scan)

    p = sub.add_parser(
        "extract_note",
        parents=[common_pretty],
        help=(
            "Open a single note from the current waterfall and extract its content. "
            "Continuation command — the daemon must already be on a search/topic_scan "
            "result page (or any waterfall containing the target card)."
        ),
    )
    p.add_argument("--note-id", required=True, help="Stable XHS note id (from a prior search_notes / topic_scan result).")
    p.add_argument("--level", choices=["card", "lite", "deep"], default="lite")
    p.add_argument("--include-media", action="store_true", help="Include OCR/vision/video media (requires backend; deep only).")
    p.set_defaults(func=_cmd_extract_note)

    p = sub.add_parser("stop", help="Stop the running tool daemon (no-op if it isn't running).")
    p.set_defaults(func=_cmd_stop)

    return parser


def main(argv: list[str] | None = None) -> int:
    args_list = list(sys.argv[1:] if argv is None else argv)
    # No arguments → fall through to the interactive REPL (legacy entry point).
    if not args_list:
        from socai.cli.repl import main as repl_main

        return repl_main()

    parser = build_parser()
    args = parser.parse_args(args_list)
    if not getattr(args, "func", None):
        parser.print_help(sys.stderr)
        return 2
    try:
        return int(args.func(args) or 0)
    except KeyboardInterrupt:
        sys.stderr.write("\n[socai] interrupted\n")
        return 130


__all__ = ["build_parser", "main"]
