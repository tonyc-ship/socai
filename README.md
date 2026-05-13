# Socai

The web-use agent.

## Setup

```bash
uv sync
```

## Desktop App

```bash
cd app
pnpm install
pnpm exec tauri dev
```

## TUI

```bash
uv run socai
```

## CLI (for Claude Code, Codex, etc)

```bash
socai search_notes "成都咖啡"                         # search + return note cards
socai topic_scan "成都咖啡" --depth standard          # search + read top notes (one bundle)
socai extract_note --note-id <id> --level lite        # open a note from the current page
socai stop                                            # stop the daemon (closes the tool tab)
```

Add `--pretty` to any tool command for indented JSON. `extract_note` is a
continuation command: a prior `search_notes` / `topic_scan` must have left the
tool tab on a waterfall containing the target card.
