"""Socai CLI package.

Splits the CLI into:
- ``runner``: headless task execution (browser session + agent loop).
  Reusable from non-CLI hosts (e.g. a future Tauri app).
- ``repl``: interactive UI — prompt, slash commands, model picker, event
  rendering. CLI-only.
"""

from socai.cli.repl import main

__all__ = ["main"]
