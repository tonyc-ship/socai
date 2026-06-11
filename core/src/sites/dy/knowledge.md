# Douyin site notes

## Tools

- `page_state`: Opens or reuses `https://www.douyin.com/` and reports the current Douyin page state, login hint, search input availability, and visible video-card count.
- `search_videos`: Searches Douyin by keyword from the homepage/search box and returns visible video cards. Passing `num_videos` scrolls search results until that many unique videos are collected or the feed stops growing.

## Current workflow

Start from the Douyin homepage, focus the search input, set the keyword, submit with Enter, wait for search results, then read video-card anchors from the page. The tool scrolls like a user to lazy-load more results.

## Notes

- The implementation uses DOM/CDP page interaction only. It does not call Douyin private APIs.
- Card fields are best-effort because Douyin frequently changes class names and text layout. `url`, `video_id`, `title`, `author`, `cover_url`, visible counts, and `raw_text` are returned when available.
