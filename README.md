# socai

专为小红书优化的 web use agent，执行小红书调研、内容抽取和自定义 agent 任务。

几点优势：
- 不使用程序化的批量爬虫，而是像人一样点击，避免被屏蔽
- 沉淀了小红书网页知识，避免agent盲目探索，又快又准
- 复用你已登录的chrome小红书账号，避免未登录被屏蔽

## Desktop App

[Download .dmg for Mac](https://github.com/tonyc-ship/socai/releases/latest/download/socai-macos-universal.dmg). 

For local development:

```bash
cd app
pnpm run dev:desktop:local
```

This writes records and artifacts to:

```text
.socai/app/tasks.json
.socai/runs/<run-dir>/
```

## CLI (for Claude Code, Codex, etc)

From the repo root:

```bash
cargo install --path cli

socai topic_scan "运营爆款思路" --num-notes 30            # 搜索并逐个获取帖子
socai search_notes "运营爆款思路"                         # 只打开搜索结果页
socai extract_note --note-id <id>                       # open a note from the current page
socai stop                                              # stop the daemon (closes the tool tab)
```

Add `--pretty` to any tool command for indented JSON.

CLI daemon telemetry sends one sanitized trace per tool command to the
first-party socai telemetry proxy (`https://socai.io/v1/events`), which forwards
to Axiom. Search query text is included by default, but can be redacted; users
can also disable telemetry entirely. The daemon also writes a local JSONL buffer
at `~/.socai/telemetry/events.jsonl` (or `$SOCAI_HOME/telemetry/events.jsonl`).

```bash
SOCAI_TELEMETRY_QUERY_TEXT=off socai topic_scan "运营爆款思路"    # keep telemetry, redact query text
SOCAI_TELEMETRY=off socai topic_scan "运营爆款思路"               # disable telemetry for this command
```

`extract_note` is a
continuation command: a prior `search_notes` / `topic_scan` must have left the
tool tab on a waterfall containing the target card.

## TUI

```bash
cargo install --path cli
socai
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

The build output is written to `site/dist/`. Deployment settings are documented in
[Website deployment](docs/website-deployment.md).

## Documentation

- [Data model](docs/data-model.md) — run artifacts, desktop task index, and timeline replay.

## 欢迎加群交流

<img src="docs/assets/wechat-group-qr.jpg" alt="socai 小红书使用 微信群二维码" width="280">
