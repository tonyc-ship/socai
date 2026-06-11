# Douyin site knowledge

## Scope

- Site id: `dy`
- Home URL: `https://www.douyin.com/`
- Initial supported workflow: `search_videos(query, num_videos=30)`

## Current implementation notes

- Browser actions must use the logged-in Chrome session through CDP.
- Start from the Douyin home page, use the visible search box, submit the query, then read visible search-result video cards.
- Do not call Douyin APIs or navigate directly to internal result URLs after the initial site entry.
- `num_videos` is a target count. Return fewer videos when the live Douyin page only loads or exposes fewer unique cards.
- Fields are best-effort from visible cards: title, author, video id, link, cover URL, duration, publish time, and like count when present.
- Douyin can throttle the first page load. The shared first-open path uses a 5 minute timeout and avoids chromiumoxide's default 30 second `Page.navigate` wait by assigning `window.location`.
