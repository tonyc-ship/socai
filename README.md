# Socai

专为小红书优化的 web use agent，执行小红书调研、内容抽取和自定义 agent 任务。

几点优势：
- 不使用程序化的批量爬虫，而是像人一样点击，避免被屏蔽
- 沉淀了小红书网页知识，避免agent盲目探索，又快又准
- 复用你已登录的chrome小红书账号，避免未登录被屏蔽

## Setup

```bash
uv sync
```

## Documentation

- [Data model](docs/data-model.md) — run artifacts, desktop task index, and timeline replay.

## Desktop App

```bash
cd app
pnpm install
pnpm exec tauri dev
```

For local development with app state and run artifacts written under the repo
instead of `~/.socai`, use:

```bash
cd app
pnpm run dev:desktop:local
```

This writes to:

```text
.socai/app/tasks.json
.socai/runs/<run-dir>/
```

## Website

The marketing/download website lives in `site/` and builds as a static Astro
site. It is separate from the desktop product UI in `app/`.

```bash
cd site
pnpm install
pnpm dev
pnpm build
```

The build output is written to `site/dist/`.

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
