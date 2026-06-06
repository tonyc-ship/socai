# socai CLI skill

Use this skill when a user asks a coding agent to research Xiaohongshu content
with the `socai` CLI.

## before you run commands

- If `socai` is missing, broken, or needs updating, read `install.md` or
  `~/.socai/share/socai/install.md` and follow the install mode runbook.
- Keep the product name lowercase: `socai`.
- `socai` drives a real browser through CDP. The user may need to log in,
  approve remote debugging, or solve a CAPTCHA manually.
- Prefer modest, targeted research requests. Do not turn `socai` into a bulk
  scraper.

## stdout and stderr contract

Parse stdout as the tool result:

- `socai search_notes ...` prints JSON to stdout.
- `socai topic_scan ...` prints JSON to stdout.
- `socai extract_note ...` prints JSON to stdout.
- `--pretty` makes stdout indented JSON for easier inspection.

Treat stderr as metadata or diagnostics:

- Successful tool commands print `run_dir: <path>` to stderr.
- Save the `run_dir` path. It contains command inputs, artifacts, and optional
  debug snapshots.
- Do not parse stderr as JSON.
- `socai stop` prints daemon status to stderr and does not produce a JSON tool
  payload.

## command selection

### `topic_scan`

Use `topic_scan` for topic research, trend review, competitive analysis, and
requests that need note bodies or comments.

Examples:

```sh
socai topic_scan "运营爆款思路" --num-notes 12 --pretty
socai topic_scan "露营装备" --tab 图文 --filter publish_time=一周内 --filter note_type=图文 --num-notes 20
```

What it does:

- Searches Xiaohongshu for the query.
- Optionally switches search tab.
- Optionally applies search-result filters.
- Reads notes in feed order up to `--num-notes`.
- Returns a JSON bundle with search cards, selected cards, notes, and skipped
  notes when prior history applies.

Use `--debug-snapshot` only when diagnosing page extraction problems because it
adds DOM, accessibility tree, and screenshot artifacts under the run directory.

### `search_notes`

Use `search_notes` for a quick first-page card scan, note discovery, or when
you need note IDs before choosing a smaller set to read.

Examples:

```sh
socai search_notes "运营爆款思路" --pretty
socai search_notes "露营装备" --filter sort=最新 --filter publish_time=一天内 --pretty
```

What it does:

- Searches Xiaohongshu like a user.
- Returns the first results page's note cards as JSON.
- Leaves the controlled tool tab on the search waterfall so a follow-up
  `extract_note` can click one of those cards.

### `extract_note`

Use `extract_note` only as a continuation command after `search_notes` or
`topic_scan` has left the current daemon-controlled tab on a search/topic
waterfall containing the target card.

Do not use `extract_note` as a standalone note fetch. If the daemon was stopped,
the tab was navigated away, or you do not have a note ID from the current search
results, run `search_notes` or `topic_scan` again first.

Example:

```sh
socai search_notes "运营爆款思路" --pretty
socai extract_note --note-id <note_id_from_card> --pretty
```

### `stop`

Use `socai stop` when:

- You are done with a session and want to close the controlled tool tab.
- You updated or replaced the `socai` binary and want to avoid stale daemon code.
- Browser automation appears stuck and you want the next command to spawn a
  fresh daemon.

Example:

```sh
socai stop || true
```

### `doctor`

Use `socai doctor` when available to inspect install mode, executable path,
daemon state, browser/CDP readiness, and update status.

Run it:

- after install or update;
- before live research if setup health is unclear;
- after browser/CDP or daemon failures;
- when the install mode appears to be `unknown`.

If the installed build does not list `doctor` in `socai --help`, do not try to
implement it during the user task. Read `install.md` and use the manual checks
there instead.

## options to remember

Common options:

- `--pretty` — print indented JSON on stdout.
- `--debug-snapshot` — record DOM, accessibility tree, and screenshots under
  `<run_dir>/snapshots/` for page-change debugging.

`topic_scan` only:

- `--num-notes <N>` — number of notes to read. Defaults to the CLI's current
  built-in value; choose a modest explicit number for repeatable research.
- `--tab <TAB>` — switch search tab. Valid labels are `全部`, `图文`, `视频`,
  and `用户`.

`topic_scan` and `search_notes`:

- `--filter <GROUP=OPTION>` — repeatable search-result filter. Omitted groups
  reset to default.

Known filter groups and options:

- `sort`: `综合`, `最新`, `最多点赞`, `最多评论`, `最多收藏`
- `note_type`: `不限`, `视频`, `图文`
- `publish_time`: `不限`, `一天内`, `一周内`, `半年内`
- `search_scope`: `不限`, `已看过`, `未看过`, `已关注`
- `distance`: `不限`, `同城`, `附近`

## workflow patterns

### broad topic research

1. Run `topic_scan` with the user's query and a modest `--num-notes`.
2. Parse stdout JSON.
3. Capture stderr and save `run_dir`.
4. Summarize findings with links or note IDs from the JSON.
5. Inspect artifacts in `run_dir` only if the JSON is not enough.

### shortlist first, then read one note

1. Run `search_notes "query" --pretty`.
2. Choose a card from stdout JSON.
3. Run `extract_note --note-id <id> --pretty` before stopping the daemon or
   changing the page context.
4. If `extract_note` says the card is unavailable, rerun `search_notes` and
   choose a note from the fresh result set.

### update or repair before use

1. Run `socai --help`.
2. Run `socai doctor` if available.
3. If install mode is `managed-binary`, follow the managed-binary update in
   `install.md`.
4. If install mode is `source-cargo`, follow the source/Cargo update in
   `install.md`.
5. If install mode is `unknown`, reinstall through a known mode rather than
   guessing how it was installed.

## browser and CDP reminders

If a command fails with a Chrome/CDP connection error:

1. Ask the user to open or approve Chrome remote debugging if manual permission
   is needed.
2. Open `chrome://inspect/#remote-debugging` in Chrome.
3. Check `http://127.0.0.1:9222/json/version` and
   `http://127.0.0.1:9223/json/version`.
4. Set one of these when Chrome is on a non-default endpoint:

   ```sh
   export SOCAI_CDP_URL="http://127.0.0.1:9222"
   export SOCAI_CDP_WS="ws://127.0.0.1:9222/devtools/browser/<id>"
   ```

5. Retry the `socai` command. If it still fails, run `socai stop || true` and
   inspect `~/.socai/rust-daemon.log`.

## output handling example

Keep stdout and stderr separate so JSON parsing is reliable:

```sh
err_file="$(mktemp)"
json_file="$(mktemp)"
if socai topic_scan "运营爆款思路" --num-notes 10 --pretty >"$json_file" 2>"$err_file"; then
  cat "$json_file"
  grep '^run_dir: ' "$err_file" >&2 || true
else
  cat "$err_file" >&2
  exit 1
fi
```

Use your agent's JSON parser on `json_file`; use `run_dir` only as artifact
metadata.
