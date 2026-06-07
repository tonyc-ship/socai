# socai

[![release](https://img.shields.io/github/v/release/socai-io/socai?style=flat-square&color=000&label=release)](https://github.com/socai-io/socai/releases/latest)
[![downloads](https://img.shields.io/github/downloads/socai-io/socai/total?style=flat-square&color=000&label=downloads)](https://github.com/socai-io/socai/releases)
[![stars](https://img.shields.io/github/stars/socai-io/socai?style=flat-square&color=000&label=stars)](https://github.com/socai-io/socai/stargazers)
[![platform](https://img.shields.io/badge/platform-macOS-000?style=flat-square)](https://github.com/socai-io/socai/releases/latest)
[![license](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-000?style=flat-square)](https://github.com/socai-io/socai/blob/main/Cargo.toml)
[![website](https://img.shields.io/badge/website-socai.io-000?style=flat-square)](https://socai.io)

专为小红书优化的 web use agent，执行小红书调研、内容抽取和自定义 agent 任务。

几点优势：
- 不使用程序化的批量爬虫，而是像人一样点击，避免被屏蔽
- 沉淀了小红书网页知识，避免agent盲目探索，又快又准
- 复用你已登录的chrome小红书账号，避免未登录被屏蔽

## CLI

socai 的核心，给 Claude Code、Codex 等 AI agent 提供开箱即用的小红书工具。

从仓库根目录安装：

```bash
cargo install --path cli
```

常用命令：

```bash
socai topic_scan "运营爆款思路" --num-notes 30 --filter publish_time=一周内   # 搜索并逐个获取帖子
socai search_notes "运营爆款思路" --filter sort=最新                          # 只打开搜索结果页
socai extract_note --note-id <id>                                          # 从当前结果页抽取某个帖子
socai stop                                                                 # 停止 daemon（关闭工具标签页）
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

## Desktop App

[Download .dmg for Mac](https://github.com/socai-io/socai/releases/latest/download/socai-macos-universal.dmg).

## 欢迎加群交流

<img src="docs/assets/wechat-group-qr.jpg" alt="socai 小红书使用 微信群二维码" width="280">
