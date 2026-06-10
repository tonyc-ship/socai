# Douyin

Douyin tools operate against the user's logged-in Chrome session on
`https://www.douyin.com/`.

## Tools

- `search_videos(query, num_videos?)`: opens Douyin if needed, uses the visible
  top search box, submits the query, then extracts video result cards from the
  search results page. It does not open video detail pages and does not read
  comments.

## Search Notes

- Prefer the high-level `search_videos` tool over manually navigating search
  URLs. It mimics the normal homepage search flow: click search input, type,
  press Enter, then read the result feed.
- Result extraction is best-effort. Stable fields are `video_id`, `url`,
  `title`, `author`, `author_url`, `cover_url`, `position`, and `raw_text`.
  Interaction counters (`likes`, `comments`, `shares`) depend on the exact
  visible card layout and may be empty.
- Passing `num_videos` scrolls the results feed until that many unique video
  links are visible or the feed stops growing.

## Limits

- The first Douyin implementation only reads search result cards. It does not
  click into video detail pages, expand captions, download media, or hydrate
  comment lists.
- If Douyin shows a verification, login wall, or blank feed, stop and report
  the visible state instead of retrying aggressively.

