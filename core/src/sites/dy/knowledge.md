Douyin / 抖音 is a Chinese short-video platform at douyin.com.

The current socai Douyin support is intentionally minimal: it registers the
site, opens/reuses a Douyin tab, and exposes a page-state probe. Do not claim
that search, video extraction, profile extraction, comments, or publishing
flows are implemented until they have been added and verified from debug
snapshots.

Available tools:

- `dy_page_state`: opens Douyin if the shared page is not already on
  douyin.com, then reports URL, title, readiness, viewport, and conservative
  login-state hints.

Development workflow:

- Use `socai dy page_state --debug-snapshot` as the first observation command.
  Inspect the generated snapshots before adding selectors or click flows.
- Add one human-like operation at a time in Rust `page.rs`; keep DOM extraction
  in `page_scripts.js`.
- Prefer stable DOM, aria, and visible-text signals. Avoid hashed class names
  unless a snapshot proves there is no better selector.
