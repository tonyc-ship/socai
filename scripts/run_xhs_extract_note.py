"""Phase-2 parity: Python half of the extract_note comparison.

Uses the existing XhsRuntime to extract a note and emits the dataclass's
to_dict() output as JSON on stdout. Pair with the Rust example:

    cargo run --example extract_note -p socai-sites -- <url> > rust.json
    uv run python scripts/run_xhs_extract_note.py <url> > python.json
    diff <(jq -S . rust.json) <(jq -S . python.json)
"""

from __future__ import annotations

import argparse
import asyncio
import json
import sys

from socai.browser.cdp import BrowserSession
from socai.browser.cdp.endpoint import discover_existing_chrome_endpoint
from socai.sites.xhs.runtime import XhsRuntime


async def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("url")
    parser.add_argument("--wait-seconds", type=float, default=8.0)
    args = parser.parse_args()

    endpoint = discover_existing_chrome_endpoint()
    if endpoint is None:
        print(
            "No CDP endpoint discovered. Open Chrome with remote debugging "
            "approved and rerun.",
            file=sys.stderr,
        )
        return 1

    browser = await BrowserSession.connect(endpoint=endpoint)
    try:
        page = await browser.new_page(args.url, wait_for_load=True)
        try:
            runtime = XhsRuntime(page)
            note = await runtime.extract_note(wait_seconds=args.wait_seconds)
            print(
                json.dumps(
                    note.to_dict(),
                    ensure_ascii=False,
                    indent=2,
                    sort_keys=True,
                )
            )
        finally:
            await browser.close_page(page.target_id)
    finally:
        await browser.stop()
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(main()))
