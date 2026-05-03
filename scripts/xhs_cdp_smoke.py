from __future__ import annotations

import argparse
import asyncio
import json
import sys
from typing import Any

from core.browser.cdp import BrowserTaskSessionManager
from core.sites.xhs import XhsRuntime
from core.sites.xhs.runtime import XHS_HOME_URL


def _compact_search(payload: dict[str, Any], *, limit: int) -> dict[str, Any]:
    cards = payload.get("cards") if isinstance(payload.get("cards"), list) else []
    return {
        "ok": payload.get("ok"),
        "query": payload.get("query"),
        "url": payload.get("url"),
        "count": payload.get("count"),
        "reason": payload.get("reason", ""),
        "submit": payload.get("submit", {}),
        "cards": cards[:limit],
    }


def _compact_note(payload: dict[str, Any]) -> dict[str, Any]:
    entity = payload.get("entity") if isinstance(payload.get("entity"), dict) else {}
    return {
        "ok": payload.get("ok"),
        "entity": {
            "note_id": entity.get("note_id", ""),
            "url": entity.get("url", ""),
            "title": entity.get("title", ""),
            "author": entity.get("author", ""),
            "content_summary": entity.get("content_summary", ""),
            "hashtags": entity.get("hashtags", []),
            "likes": entity.get("likes", ""),
            "favorites": entity.get("favorites", ""),
            "comments_count": entity.get("comments_count", ""),
        },
    }


async def run(args: argparse.Namespace) -> int:
    manager = BrowserTaskSessionManager(
        browser_ws_url=args.cdp_ws,
        http_url=args.cdp_url,
        inspect_timeout=args.inspect_timeout,
        on_event=lambda message: print(f"[socai] {message}", file=sys.stderr),
    )
    try:
        task = await manager.create_task(start_url=XHS_HOME_URL, label=f"xhs:{args.query}", site="xiaohongshu")
        runtime = XhsRuntime(task.page)
        search = await runtime.search_notes(args.query, wait_seconds=args.wait_seconds)
        result: dict[str, Any] = {"search": _compact_search(search, limit=args.limit)}

        if search.get("ok") and search.get("cards"):
            note = await runtime.read_note(index=0)
            result["note"] = _compact_note(note)

        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0 if result.get("note", {}).get("ok") else 1
    finally:
        # The one-shot smoke CLI exits after one task. The app backend should
        # keep its BrowserTaskSessionManager alive between user tasks.
        await manager.shutdown()


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Run a live Xiaohongshu CDP smoke task.")
    parser.add_argument("query", help="Xiaohongshu search query.")
    parser.add_argument("--cdp-ws", help="Browser websocket URL. Defaults to SOCAI_CDP_WS.")
    parser.add_argument("--cdp-url", help="HTTP DevTools URL, for example http://127.0.0.1:9222. Defaults to SOCAI_CDP_URL.")
    parser.add_argument("--inspect-timeout", type=float, default=45.0, help="Seconds to wait after opening chrome://inspect.")
    parser.add_argument("--wait-seconds", type=float, default=4.0, help="Wait for search transition. Default: 4.")
    parser.add_argument("--limit", type=int, default=5, help="Number of cards to print. Default: 5.")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        return asyncio.run(run(args))
    except Exception as exc:  # noqa: BLE001 - command-line diagnostic
        print(f"Live XHS CDP smoke failed: {exc}", file=sys.stderr)
        print(
            "Default behavior reuses your existing logged-in Chrome profile. "
            "If Chrome asks for remote debugging permission, approve it and rerun. "
            "You can also override CDP with --cdp-ws/--cdp-url.",
            file=sys.stderr,
        )
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
