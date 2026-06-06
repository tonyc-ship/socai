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

socai topic_scan "运营爆款思路" --num-notes 30 --filter publish_time=一周内   # 搜索并逐个获取帖子
socai search_notes "运营爆款思路" --filter sort=最新                          # 只打开搜索结果页
socai extract_note --note-id <id>                                          # open a note from the current page
socai stop                                                                 # stop the daemon (closes the tool tab)
```

Options:

- `--filter <GROUP=OPTION>` — search-result filter, repeatable
  (`topic_scan` & `search_notes`). Groups & options:
  `sort` (综合/最新/最多点赞/最多评论/最多收藏), `note_type` (不限/视频/图文),
  `publish_time` (不限/一天内/一周内/半年内), `search_scope` (不限/已看过/未看过/已关注),
  `distance` (不限/同城/附近). Omitted groups reset to default.
  e.g. `--filter publish_time=一天内 --filter note_type=图文`
- `--tab <TAB>` — search tab to switch to, `topic_scan` only (`全部` / `图文` / `视频` / `用户`).
- `--num-notes <N>` — notes to read, `topic_scan` only (scrolls only if the first page holds fewer).
- `--pretty` — indented JSON (any tool command).
- `--debug-snapshot` — record DOM + a11y tree + screenshots per page change.

`extract_note` is a
continuation command: a prior `search_notes` / `topic_scan` must have left the
tool tab on a waterfall containing the target card.

CLI telemetry is enabled by default, and search query text is included by
default. Use these environment variables when you want to redact query text or
disable telemetry for a command:

```bash
SOCAI_TELEMETRY_QUERY_TEXT=off socai topic_scan "运营爆款思路"    # keep telemetry, redact query text
SOCAI_TELEMETRY=off socai topic_scan "运营爆款思路"               # disable telemetry for this command
```

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
