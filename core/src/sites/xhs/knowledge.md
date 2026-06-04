# Xiaohongshu Site Knowledge

Xiaohongshu / 小红书 / XHS is a Chinese lifestyle social platform at
https://www.xiaohongshu.com (default landing page:
https://www.xiaohongshu.com/explore). Posts are called notes (笔记) and are
usually image carousels or short videos with title, body text, hashtags,
engagement counts, comments, and author/profile context.

## Browser Lock

The browser is locked to Xiaohongshu; every task is an XHS task. Do not
navigate to other websites. Drive the site through the XHS tools below
instead of direct `/explore/<note_id>` navigation, which often triggers
blocked, blank, app-only, or QR-code flows. Reply in the same language as the
user's task and ground your answer in tool output.

## Tools

`search_notes`, `extract_search_cards`, `list_search_tabs`,
`click_search_tab`, `open_note`, `close_note`, `read_note`, `extract_note`,
`extract_comments`, `scroll_in_note`, `collect_carousel_images`,
`extract_profile`, `topic_scan`, `page_state`.

Prefer `topic_scan` for any "research a topic" task — it bundles search,
sample, and read in one call. For one-off lookups: `page_state` →
`search_notes` → `read_note` (or `open_note` + `extract_note` +
`extract_comments`). Close any open note modal before searching again.

## Anti-Bot Rules

- Prefer `search_notes` from the homepage or any non-XHS tab; it navigates
  to XHS and submits the search like a user.
- Do not navigate directly to `/explore/<note_id>` unless no card-click path is
  available. Open notes from search/profile cards with `read_note`.
- Close note modals with `close_note`, Escape, or the close button. Do not
  reload the page just to close a note.
- If a page shows QR/app-only prompts, captcha, security verification, 404/blank
  direct-detail routes, or "page unavailable" copy, stop retrying that URL and
  return to search/profile card clicks.
- Add screenshots when visual state or extraction confidence matters.

## Page States

- `homepage`: left navigation and top search input. Use `search_notes`.
- `search_results`: query input, tabs `全部` / `图文` / `视频` / `用户`, waterfall
  note cards with cover, title, author, likes, and type.
- `note_detail`: modal or full detail. Left side is carousel/video, right side
  is author, title/body, hashtags, comments, and engagement bar.
- `profile_page`: author avatar/name/XHS ID/bio/stats and note-card grid. Use
  `extract_profile`; scroll loads more cards.

## Entity Fields

Note fields: `note_id`, `url`, `type`, `title`, `author`, `author_id`,
`author_url`, `content`, `hashtags`, `date`, `location`, `ip_location`,
`likes`, `favorites`, `comments_count`, `shares`, `image_count`, `images`,
`video`, `top_comments`.

Comment fields: `username`, `text`, `likes`, `like_count`, `time`,
`is_author_reply`, `is_pinned`, `reply_count`, `sub_comments`.

Author fields: `display_name`, `xhs_id`, `profile_url`, `bio`, `followers`,
`following`, `likes_and_collections`, `note_cards`.

Image fields: `url`, `index`, `is_cover`, optional `ocr_text`,
`vision_description`, `local_path`.

Video fields: `url`, `resolved_url`, `poster_url`, optional `transcript`,
`transcript_summary`, `frame_paths`, `frame_descriptions`, `visual_summary`.

## Workflows

- Topic research: call `topic_scan(query=..., num_notes=N)`. It
  searches, optionally switches tab, optionally applies search-result filters,
  then reads notes top-to-bottom in feed order — opening each (which pages the
  next cards in as it scrolls), reading its body + top comments, writing
  artifacts, closing note modals, and marking already analyzed posts. Default
  `num_notes` is 10.
- Quick breadth scan: use `search_notes` (atomic — returns the first results
  page's cards, no scrolling) or `extract_search_cards` to inspect cards
  without opening notes.
- Manual note read: use `read_note(index=N)` or `read_note(note_id=...)`.
  Use `level="card"` for metadata only, `level="lite"` for body/comments, and
  `level="deep"` plus `include_media=true` only when images, OCR, video, or
  visual evidence materially matters.
- Creator analysis: navigate/open a profile, then use `extract_profile`.
  Keep creator inventory/style analysis separate from keyword/topic sampling
  unless the user explicitly asks for both.
- Comment sentiment: use lite/deep note reads first; call `scroll_in_note`
  then `extract_comments` when more visible comments are needed.
- Media-heavy tasks: use deep reads sparingly. OCR, vision, transcription, and
  frame extraction depend on optional local/cloud capabilities and can be slow.

## Reading Levels

- `card`: note metadata and engagement only. Low latency; no comments/media.
- `lite`: title, author, body, hashtags, publish metadata, engagement, and a
  small hot-comment sample. Best default for most search/research tasks.
- `deep`: lite fields plus optional image OCR/vision, video transcript, sampled
  frame descriptions, and a larger comment sample. Use only when the user asks
  for media evidence, screenshots/OCR, video narration, or high-confidence
  evidence.

## Evidence Rules

- Preserve real XHS post links from cards when available, including
  `xsec_token` query parameters. Fall back to bare `/explore/<note_id>` only
  when no real tokenized URL exists.
- Cards carry `already_analyzed` / `history_level` / `history_include_media`
  flags when a prior run already read them. `read_note` and `topic_scan`
  short-circuit notes already covered at the requested level and media
  setting — the returned payload has `skipped: true` plus the prior
  `history` entry. To deepen prior analysis, request a higher `level` (e.g.
  `deep` after a `lite`) or set `include_media: true`.
- Keep DOM text, comment evidence, image OCR/vision, and video transcript/frame
  evidence labeled separately in final answers.
- If a read returns a stale-note warning or note-id mismatch, close the current
  modal and reopen the intended card before trusting the result.
- Post-level engagement extraction must exclude comment-area DOM; otherwise
  comment likes or comment UI text can be misread as the note's own engagement.
- For reports, include screenshots and artifact paths only when they support the
  conclusion or make verification easier.

## Chinese UI Hints

- Search tabs: `全部`, `图文`, `视频`, `用户`.
- No-result copy often contains: `没有找到相关内容`, `换个词试试`, `暂无相关内容`.
- Engagement labels: `赞`, `收藏`, `评论`, `分享`.
- Count suffixes: `万` and `w` mean 10000; `k` means 1000.
