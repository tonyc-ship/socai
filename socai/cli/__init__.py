"""Socai CLI package.

Module map:
- ``commands``: argparse dispatcher and tool subcommands (``socai search_notes`` …).
- ``daemon``: long-lived process owning a browser tool tab.
- ``daemon_client``: client helpers for talking to the daemon.
- ``repl``: interactive prompt-toolkit UI (no-args entry point).
- ``runner``: headless task runner reused by both the REPL and a future Tauri host.
"""

from socai.cli.commands import main

__all__ = ["main"]
