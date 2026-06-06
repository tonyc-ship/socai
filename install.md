# socai CLI install runbook for coding agents

This file is the agent-facing setup, update, and troubleshooting runbook for the
`socai` CLI. Prefer it over README snippets when a user asks a coding agent to
set up or repair `socai`.

## scope and invariants

- Keep the product name lowercase: `socai`.
- Install only the CLI and its agent docs here. Do not update README or site
  docs as part of this runbook.
- Normal managed-binary installs must not require Rust or Cargo.
- Use the source/Cargo fallback only when no compatible release binary exists,
  the platform is unsupported, or the user explicitly asks for a source or
  development install.
- Do not implement missing commands while installing. In particular, `socai
  doctor`, daemon version handshake, and release packaging are tracked by
  separate issues. Use the best available released CLI and the manual checks
  below when those features are not present yet.

## install modes

Agents should keep the user's install in one of these modes:

- `managed-binary` — preferred when a compatible GitHub Release CLI asset exists.
  The stable executable path is `~/.socai/bin/socai`. Agent docs are copied to
  `~/.socai/share/socai/SKILL.md` and `~/.socai/share/socai/install.md`.
- `source-cargo` — fallback for unsupported platforms or explicit development
  installs. Clone the repo to a durable path, update with `git pull --ff-only`,
  and install with `cargo install --path cli --force`.
- `unknown` — diagnostic state for any ambiguous executable path or install that
  cannot be tied to the two known modes. Do not guess an update procedure;
  reinstall through `managed-binary` or `source-cargo`.

Quick local mode check:

```sh
socai_path="$(command -v socai || true)"
case "$socai_path" in
  "$HOME/.socai/bin/socai") echo managed-binary ;;
  "$HOME/.cargo/bin/socai") echo source-cargo ;;
  "") echo missing ;;
  *) echo unknown ;;
esac
```

## choose an install path

1. Detect the platform:

   ```sh
   uname -s
   uname -m
   ```

2. On macOS (`Darwin`, `arm64` or `x86_64`), try `managed-binary` first using
   the latest GitHub Release asset named `socai-cli-macos-universal.tar.gz` and
   its checksum file `socai-cli-macos-universal.tar.gz.sha256`.
3. If that asset is missing, the download fails, or the platform is not covered
   by a release asset, use `source-cargo`.
4. On Windows, distinguish WSL from native Windows:
   - WSL: use the Linux/source fallback unless a verified release asset exists.
   - Native Windows: treat as unverified until issue #77 is resolved. The
     current daemon path uses Unix sockets, so prefer WSL or another verified
     environment instead of promising native Windows support.
5. If an existing `socai` is on an unexpected path, treat it as `unknown` and
   reinstall through a known mode. Avoid deleting unknown binaries unless the
   user explicitly approves.

## managed-binary install: macOS universal

Use this path only when the GitHub Release contains a compatible CLI asset.
This path does not use Rust or Cargo.

```sh
set -euo pipefail

repo="tonyc-ship/socai"
asset="socai-cli-macos-universal.tar.gz"
checksum="${asset}.sha256"
prefix="${SOCAI_PREFIX:-$HOME/.socai}"
bin_dir="$prefix/bin"
share_dir="$prefix/share/socai"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

base_url="https://github.com/${repo}/releases/latest/download"
archive="$tmp_dir/$asset"
checksum_file="$tmp_dir/$checksum"
unpack_dir="$tmp_dir/unpack"

curl -fL "$base_url/$asset" -o "$archive"
curl -fL "$base_url/$checksum" -o "$checksum_file"
(cd "$tmp_dir" && shasum -a 256 -c "$checksum")

mkdir -p "$unpack_dir" "$bin_dir" "$share_dir"
tar -xzf "$archive" -C "$unpack_dir"

if [ ! -f "$unpack_dir/socai" ]; then
  echo "release archive did not contain ./socai" >&2
  exit 1
fi

install -m 0755 "$unpack_dir/socai" "$bin_dir/socai"
[ -f "$unpack_dir/SKILL.md" ] && install -m 0644 "$unpack_dir/SKILL.md" "$share_dir/SKILL.md"
[ -f "$unpack_dir/install.md" ] && install -m 0644 "$unpack_dir/install.md" "$share_dir/install.md"
[ -f "$unpack_dir/manifest.json" ] && install -m 0644 "$unpack_dir/manifest.json" "$share_dir/manifest.json"

export PATH="$bin_dir:$PATH"
"$bin_dir/socai" --help
```

If the asset does not exist in the latest release, do not download the desktop
`.dmg` as a substitute for the CLI. Use `source-cargo` instead. The dedicated
CLI release asset is tracked separately by issue #67.

Make `~/.socai/bin` persistent in the user's shell if it is not already on
`PATH`:

```sh
case ":$PATH:" in
  *":$HOME/.socai/bin:"*) ;;
  *)
    shell_rc="$HOME/.zshrc"
    [ -n "${BASH_VERSION:-}" ] && shell_rc="$HOME/.bashrc"
    printf '\n# socai CLI\nexport PATH="$HOME/.socai/bin:$PATH"\n' >> "$shell_rc"
    ;;
esac
```

If macOS blocks the binary because it was downloaded from the internet, confirm
with the user before changing quarantine metadata, then run:

```sh
xattr -d com.apple.quarantine "$HOME/.socai/bin/socai" 2>/dev/null || true
```

## source-cargo fallback install

Use this path when no compatible release binary exists, the user asked for a
source/development install, or the platform is not yet covered by a managed
binary. This path requires Git plus Rust/Cargo. Ask before installing missing
system dependencies if the session policy requires permission.

```sh
set -euo pipefail

src_root="${SOCAI_SOURCE_ROOT:-$HOME/.socai/src}"
repo_dir="$src_root/socai"
mkdir -p "$src_root"

if [ -d "$repo_dir/.git" ]; then
  git -C "$repo_dir" pull --ff-only
else
  git clone https://github.com/tonyc-ship/socai.git "$repo_dir"
fi

cd "$repo_dir"
cargo install --path cli --force

mkdir -p "$HOME/.socai/share/socai"
install -m 0644 SKILL.md "$HOME/.socai/share/socai/SKILL.md"
install -m 0644 install.md "$HOME/.socai/share/socai/install.md"

export PATH="$HOME/.cargo/bin:$PATH"
socai --help
```

Make `~/.cargo/bin` persistent if needed:

```sh
case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) ;;
  *)
    shell_rc="$HOME/.zshrc"
    [ -n "${BASH_VERSION:-}" ] && shell_rc="$HOME/.bashrc"
    printf '\n# cargo-installed CLIs\nexport PATH="$HOME/.cargo/bin:$PATH"\n' >> "$shell_rc"
    ;;
esac
```

## skill registration

Always leave a canonical copy at `~/.socai/share/socai/SKILL.md`. Then register
or point the user's coding agent at that file.

### Claude Code

Preferred user-level registration:

```sh
mkdir -p "$HOME/.claude/skills/socai"
cp "$HOME/.socai/share/socai/SKILL.md" "$HOME/.claude/skills/socai/SKILL.md"
```

If the user wants project-local registration instead, copy the same file to the
project's `.claude/skills/socai/SKILL.md` after confirming that project-local
agent instructions should be changed.

### Codex

Use a durable copy plus an instruction pointer. Codex versions differ in how
they discover skills, so do both when possible:

```sh
mkdir -p "$HOME/.codex/skills/socai"
cp "$HOME/.socai/share/socai/SKILL.md" "$HOME/.codex/skills/socai/SKILL.md"
mkdir -p "$HOME/.codex"
touch "$HOME/.codex/AGENTS.md"
```

Append this section to `~/.codex/AGENTS.md` if it is not already present:

```md
## socai

When asked to use the socai CLI for Xiaohongshu research, read
`~/.socai/share/socai/SKILL.md` first and follow its stdout/stderr contract.
```

Do not overwrite unrelated Codex instructions.

## update runbooks

### update `managed-binary`

1. Download the latest `socai-cli-macos-universal.tar.gz` and checksum exactly
   as in the managed-binary install steps.
2. Verify the checksum when the `.sha256` file is available.
3. Replace `~/.socai/bin/socai` and the docs in `~/.socai/share/socai/`.
4. Stop stale daemon state:

   ```sh
   socai stop || true
   ```

5. Run diagnostics:

   ```sh
   if socai --help | grep -q 'doctor'; then
     socai doctor
   else
     socai --help
   fi
   ```

   If this installed build does not yet list `doctor` in `socai --help`, use the
   manual checks in this file and run `socai --help` as the minimum validation.

### update `source-cargo`

```sh
set -euo pipefail
repo_dir="${SOCAI_SOURCE_REPO:-$HOME/.socai/src/socai}"
git -C "$repo_dir" pull --ff-only
cd "$repo_dir"
cargo install --path cli --force
mkdir -p "$HOME/.socai/share/socai"
install -m 0644 SKILL.md "$HOME/.socai/share/socai/SKILL.md"
install -m 0644 install.md "$HOME/.socai/share/socai/install.md"
socai stop || true
if socai --help | grep -q 'doctor'; then
  socai doctor
else
  socai --help
fi
```

If `socai doctor` is not available in the installed build, validate with
`socai --help` and the browser/CDP checks below. Once daemon version handshake
exists, normal commands should restart mismatched daemons automatically; until
then, keep `socai stop || true` in the update path.

### repair `unknown`

1. Record the current path and help output for the user:

   ```sh
   command -v socai || true
   socai --help || true
   ```

2. Do not infer the update mechanism from an unknown path.
3. Pick a known install path using the decision tree above.
4. Install through `managed-binary` or `source-cargo`.
5. Run `socai stop || true`, then `socai doctor` when available.

## browser and CDP troubleshooting

`socai` controls the user's browser through the Chrome DevTools Protocol (CDP).
The user may need to be logged into Xiaohongshu in the controlled browser.
Manual browser permission, login, and CAPTCHA steps are user actions; ask the
user to complete those when required.

Useful checks and fixes:

1. Open Chrome's remote-debugging page:

   ```sh
   open -a "Google Chrome" 'chrome://inspect/#remote-debugging'
   ```

   On non-macOS systems, open `chrome://inspect/#remote-debugging` in Chrome or
   a compatible Chromium browser.

2. Check common local CDP ports:

   ```sh
   curl -fsS http://127.0.0.1:9222/json/version || true
   curl -fsS http://127.0.0.1:9223/json/version || true
   ```

3. If Chrome uses a non-default debug endpoint, set one of:

   ```sh
   export SOCAI_CDP_URL="http://127.0.0.1:9222"
   export SOCAI_CDP_WS="ws://127.0.0.1:9222/devtools/browser/<id>"
   ```

   `SOCAI_CDP_WS` points directly at the browser websocket. `SOCAI_CDP_URL`
   points at the HTTP endpoint whose `/json/version` response contains the
   websocket URL.

4. If no debug endpoint is running, start Chrome with a remote-debugging port.
   A separate user-data directory avoids profile lock conflicts but may require
   the user to log in again:

   ```sh
   /Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
     --remote-debugging-port=9222 \
     --user-data-dir="$HOME/.socai/chrome-debug-profile"
   ```

5. If commands still fail, inspect daemon files:

   ```sh
   ls -la "$HOME/.socai"
   tail -n 200 "$HOME/.socai/rust-daemon.log" 2>/dev/null || true
   socai stop || true
   ```

## CLI stdout and stderr contract

Agent code should parse tool output from stdout only:

- `search_notes`, `topic_scan`, and `extract_note` print JSON to stdout.
- `--pretty` changes formatting but stdout remains JSON.
- `run_dir: ...` is printed to stderr as artifact metadata, not as JSON.
- `stop` prints daemon status to stderr.

When capturing output, keep stdout and stderr separate. Record `run_dir` so the
agent can inspect command inputs, artifacts, and optional debug snapshots later.

## command smoke checks

Use non-destructive checks first:

```sh
socai --help
socai search_notes --help
socai topic_scan --help
socai extract_note --help
```

Only run live Xiaohongshu commands after browser/CDP is ready and the user is
comfortable using their logged-in browser session.
