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

The CLI is intended for coding agents first. For setup, give your agent the
install runbook and skill instead of starting with a source build. The agent
should install or update `socai`, register `SKILL.md`, and run `socai doctor`
when the installed build supports it:

- [install.md](install.md) — setup, update, and troubleshooting runbook for
  agents.
- [SKILL.md](SKILL.md) — usage contract and command guide agents should read
  before running `socai`.

Copy/paste prompt for Claude Code or Codex:

```text
Install or update the socai CLI on this machine from https://github.com/tonyc-ship/socai.
Read the repo's top-level install.md and SKILL.md first.
Prefer a compatible GitHub Release CLI binary; use the source/Cargo fallback only when no compatible binary exists, the release download/checksum fails, or I ask for a source/development install.
Do not use the desktop .dmg as a CLI substitute.
Register the socai skill for this agent, run socai doctor when available (otherwise run socai --help and the manual checks in install.md), then report the install mode, command -v socai, and validation result.
```

### install modes

- **Release binary (preferred when compatible):** agents should use the latest
  GitHub Release CLI asset when it matches the user's OS and CPU. The current
  managed macOS CLI asset is
  [`socai-cli-macos-universal.tar.gz`](https://github.com/tonyc-ship/socai/releases/latest/download/socai-cli-macos-universal.tar.gz)
  with checksum
  [`socai-cli-macos-universal.tar.gz.sha256`](https://github.com/tonyc-ship/socai/releases/latest/download/socai-cli-macos-universal.tar.gz.sha256);
  it installs to `~/.socai/bin/socai` and does not require Rust or Cargo.
- **Source/Cargo fallback:** use this when no compatible CLI binary exists, the
  release asset cannot be verified, the platform is not covered by a release
  binary, or a development/source install is requested. This path requires Git
  plus Rust/Cargo and keeps the source checkout in a durable location.

Do not assume every platform has a prebuilt CLI binary. Linux, WSL, native
Windows, and future targets should follow the compatibility checks in
[install.md](install.md) and fall back to source/Cargo when needed.

### source/development fallback

From a source checkout:

```bash
cargo install --path cli --force
mkdir -p "$HOME/.socai/share/socai"
install -m 0644 SKILL.md "$HOME/.socai/share/socai/SKILL.md"
install -m 0644 install.md "$HOME/.socai/share/socai/install.md"
socai stop || true
if socai --help | grep -q 'doctor'; then socai doctor; else socai --help; fi
```

### usage examples

After install or update, run diagnostics first when available:

```bash
if socai --help | grep -q 'doctor'; then socai doctor; fi                  # inspect install/browser/daemon health when the build includes doctor
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

`extract_note` is a continuation command: a prior `search_notes` / `topic_scan`
must have left the tool tab on a waterfall containing the target card.

For updates or troubleshooting, rerun the agent setup prompt or follow the
managed-binary, source/Cargo, and browser/CDP repair sections in
[install.md](install.md). Stop stale daemon state with `socai stop || true`, then
run `socai doctor` when the installed build supports it; older builds should use
`socai --help` and the manual checks in `install.md`.

## TUI

After installing the CLI through a release binary or source fallback:

```bash
socai
```

For local TUI development, use the source/development fallback above.

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
