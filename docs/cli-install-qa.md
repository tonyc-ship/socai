# socai CLI install/update QA runbook

Tracking: issue #71, part of epic #65.

Use this runbook to smoke-test the agent-first CLI install and update modes before
closing issue #71. It is intentionally evidence-oriented: every tester should
record exact commands, outputs, paths, release URLs, and limitations so the final
release verification can be reviewed independently.

## current status and release gate

Do not mark issue #71 complete until all final release gates in this section pass.

- Managed-binary macOS verification is currently **blocked** until the GitHub
  Release CLI assets exist in `latest`:
  - `socai-cli-macos-universal.tar.gz`
  - `socai-cli-macos-universal.tar.gz.sha256`
- As of 2026-06-06, the latest published release checked with
  `gh release view --repo tonyc-ship/socai` is `v0.1.5` and exposes only
  `socai-macos-universal.dmg`. Do not use the desktop `.dmg` as a CLI
  substitute.
- The release workflow that creates the CLI tarball must be merged and run
  before a clean-machine managed-binary pass can be claimed.
- Native Windows daemon IPC is not a verified path yet because the CLI daemon
  uses Unix-domain sockets; native Windows support is tracked by issue #80.
- If the tested build does not expose `socai doctor`, record that as a blocker
  for final #71 acceptance and use the manual checks in `install.md` only as
  fallback evidence.

Final release gates for #71:

| gate | required result | evidence to attach |
| --- | --- | --- |
| macOS managed-binary clean-machine install | pass from GitHub Release CLI tarball without installing Rust/Cargo | release asset URLs, checksum output, `command -v socai`, `socai --version`, `socai doctor`, PATH evidence |
| source/Cargo fallback install and update | pass from durable checkout | repo path, `git rev-parse HEAD`, `cargo install` result, `command -v socai`, `socai --version`, `socai doctor` or documented fallback |
| skill registration | pass for Claude Code and Codex paths, or documented N/A | copied file paths, `cmp`/`test` output, Codex instruction pointer |
| browser/CDP troubleshooting | actionable and validated where possible | CDP endpoint checks, env vars used, daemon log path, user-action notes |
| stale daemon / version mismatch handling | pass or documented limitation | old/new versions, daemon state before/after update, `socai stop`/doctor output |
| WSL/native Windows | WSL source/Cargo pass if available; native Windows limitation documented | WSL distro/path evidence or issue #80 limitation note |

## tester evidence header

Copy this block into the PR comment, issue comment, or QA notes for each run.

```text
QA run id:
Tester:
Date/time (UTC):
Issue/PR under test:
GitHub Release tag and URL:
Install mode tested: managed-binary | source-cargo | WSL source-cargo | native Windows note
Host OS/version:
CPU architecture:
Shell:
Clean machine or existing machine:
Rust/Cargo present before test: yes | no | not checked
Existing socai before test: command -v output and version/help output
Browser used for CDP:
Network/proxy/VPN notes:
Result: pass | fail | blocked | partial
Summary:
Evidence artifact links or pasted output:
Residual risks:
```

## preflight checks for every run

Run these before choosing an install mode.

```sh
set -u

date -u '+%Y-%m-%dT%H:%M:%SZ'
uname -a 2>/dev/null || true
printf 'shell=%s\n' "${SHELL:-unknown}"
printf 'PATH=%s\n' "$PATH"
command -v socai || true
socai --version 2>&1 || true
socai --help 2>&1 | sed -n '1,80p' || true
command -v cargo || true
cargo --version 2>&1 || true
```

Record the detected install mode when `socai` already exists:

```sh
socai_path="$(command -v socai || true)"
case "$socai_path" in
  "$HOME/.socai/bin/socai") echo 'install_mode=managed-binary' ;;
  "$HOME/.cargo/bin/socai") echo 'install_mode=source-cargo' ;;
  "") echo 'install_mode=missing' ;;
  *) echo "install_mode=unknown path=$socai_path" ;;
esac
```

If the path is `unknown`, do not infer an update path. Record the path and
reinstall through a known mode.

## managed-binary macOS clean-machine smoke checklist

Use this checklist only after the latest GitHub Release publishes the CLI
tarball and checksum. A valid clean-machine pass must not require Rust or Cargo.
Prefer a fresh macOS VM, a clean physical Mac, or a throwaway user account with
no previous `~/.socai` state.

### macOS environment evidence

```sh
set -euo pipefail

sw_vers
uname -m
command -v cargo >/dev/null 2>&1 && cargo --version || echo 'cargo_not_installed'
command -v rustc >/dev/null 2>&1 && rustc --version || echo 'rustc_not_installed'
command -v socai || true
ls -la "$HOME/.socai" 2>/dev/null || true
```

Expected for the strict clean-machine gate:

- `cargo_not_installed` and `rustc_not_installed`, or an explanation if the
  host cannot be made Cargo-free.
- No existing `socai`, or exact pre-existing state recorded and removed in a
  throwaway environment before the install.

### release asset availability

```sh
set -euo pipefail

gh release view --repo tonyc-ship/socai --json tagName,url,assets \
  --jq '{tagName, url, assets: [.assets[].name]}'

repo='tonyc-ship/socai'
asset='socai-cli-macos-universal.tar.gz'
checksum="${asset}.sha256"
base_url="https://github.com/${repo}/releases/latest/download"

curl -fI "$base_url/$asset"
curl -fI "$base_url/$checksum"
```

Expected:

- Both `curl -fI` commands return success.
- The asset list includes the CLI tarball and `.sha256` file.
- If either asset is missing, stop this checklist and mark managed-binary QA as
  `blocked`. Do not install the desktop `.dmg` as a CLI substitute.

### download, verify, and inspect the archive

```sh
set -euo pipefail

repo='tonyc-ship/socai'
asset='socai-cli-macos-universal.tar.gz'
checksum="${asset}.sha256"
base_url="https://github.com/${repo}/releases/latest/download"
tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT

curl -fL "$base_url/$asset" -o "$tmp_dir/$asset"
curl -fL "$base_url/$checksum" -o "$tmp_dir/$checksum"
(cd "$tmp_dir" && shasum -a 256 -c "$checksum")
tar -tzf "$tmp_dir/$asset" | sort
```

Expected archive contents:

```text
SKILL.md
install.md
manifest.json
socai
```

Record the checksum output and archive listing. If `manifest.json` is present,
record it:

```sh
tar -xOzf "$tmp_dir/$asset" manifest.json | tee "$tmp_dir/manifest.json"
```

### install to the managed path and verify PATH behavior

```sh
set -euo pipefail

prefix="${SOCAI_PREFIX:-$HOME/.socai}"
bin_dir="$prefix/bin"
share_dir="$prefix/share/socai"
unpack_dir="$tmp_dir/unpack"
mkdir -p "$unpack_dir" "$bin_dir" "$share_dir"

tar -xzf "$tmp_dir/$asset" -C "$unpack_dir"
install -m 0755 "$unpack_dir/socai" "$bin_dir/socai"
install -m 0644 "$unpack_dir/SKILL.md" "$share_dir/SKILL.md"
install -m 0644 "$unpack_dir/install.md" "$share_dir/install.md"
[ -f "$unpack_dir/manifest.json" ] && install -m 0644 "$unpack_dir/manifest.json" "$share_dir/manifest.json"

export PATH="$bin_dir:$PATH"
hash -r 2>/dev/null || true
command -v socai
type -a socai || true
test "$(command -v socai)" = "$HOME/.socai/bin/socai"
socai --version
socai --help | sed -n '1,120p'
```

Expected:

- `command -v socai` is exactly `$HOME/.socai/bin/socai`.
- `type -a socai` does not show an earlier conflicting path before
  `$HOME/.socai/bin/socai`.
- `socai --version` prints the release version under test.
- `socai --help` lists the expected CLI commands. For final #71 acceptance, it
  must include `doctor`; if it does not, mark the final managed-binary gate as
  failed or blocked.

Make PATH persistent only after confirming the transient PATH test passes:

```sh
case ":$PATH:" in
  *":$HOME/.socai/bin:"*) echo 'managed binary path already active' ;;
  *)
    shell_rc="$HOME/.zshrc"
    [ -n "${BASH_VERSION:-}" ] && shell_rc="$HOME/.bashrc"
    printf '\n# socai CLI\nexport PATH="$HOME/.socai/bin:$PATH"\n' >> "$shell_rc"
    echo "updated $shell_rc"
    ;;
esac
```

Open a new shell and re-run:

```sh
command -v socai
socai --version
```

### doctor and no-Cargo assertion

```sh
set -euo pipefail

socai doctor
command -v cargo >/dev/null 2>&1 && echo 'cargo_present_after_install' || echo 'cargo_still_absent'
```

Expected for final #71 acceptance:

- `socai doctor` exits successfully and reports enough install/browser/daemon
  state for an agent to recover from common setup problems.
- If the machine started without Cargo/Rust, it remains without Cargo/Rust after
  the managed-binary install. If `cargo_present_after_install` appears, explain
  why it was present before the test or mark the no-Cargo gate failed.

### managed-binary update smoke

Run this on a machine that already has an older managed-binary `socai` installed
or simulate with the previous release once CLI release assets exist.

```sh
set -euo pipefail

before_path="$(command -v socai || true)"
before_version="$(socai --version 2>&1 || true)"
printf 'before_path=%s\nbefore_version=%s\n' "$before_path" "$before_version"
socai stop || true

# Re-run the managed-binary download, checksum, unpack, and install steps above.

after_path="$(command -v socai)"
after_version="$(socai --version 2>&1)"
printf 'after_path=%s\nafter_version=%s\n' "$after_path" "$after_version"
socai doctor
```

Expected:

- Path remains `$HOME/.socai/bin/socai`.
- Version changes to the release under test.
- Stale daemon state is stopped or reported in a way that an agent can recover
  from.

## source/Cargo fallback smoke checklist

Use this path when no compatible release CLI asset exists, the platform is not
covered by a managed binary, or the user requests a source/development install.
This path is expected to require Git plus Rust/Cargo.

### prerequisites

```sh
set -euo pipefail

command -v git
git --version
command -v cargo
cargo --version
command -v rustc
rustc --version
```

If a dependency is missing, follow `install.md` for the platform and record any
system package manager commands separately.

### install from a durable checkout

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
git status --short
git rev-parse HEAD
cargo install --path cli --force

mkdir -p "$HOME/.socai/share/socai"
install -m 0644 SKILL.md "$HOME/.socai/share/socai/SKILL.md"
install -m 0644 install.md "$HOME/.socai/share/socai/install.md"

export PATH="$HOME/.cargo/bin:$PATH"
hash -r 2>/dev/null || true
command -v socai
type -a socai || true
test "$(command -v socai)" = "$HOME/.cargo/bin/socai"
socai --version
socai --help | sed -n '1,120p'
if socai --help | grep -q 'doctor'; then
  socai doctor
else
  echo 'doctor_missing_use_manual_install_md_checks'
fi
```

Expected:

- `command -v socai` is exactly `$HOME/.cargo/bin/socai`.
- `socai --version` reflects the source version installed.
- For final #71 acceptance, the tested source build should include
  `socai doctor`; if it does not, record the missing command as a blocker and
  include manual fallback evidence from `install.md`.

Make PATH persistent if needed:

```sh
case ":$PATH:" in
  *":$HOME/.cargo/bin:"*) echo 'cargo bin path already active' ;;
  *)
    shell_rc="$HOME/.zshrc"
    [ -n "${BASH_VERSION:-}" ] && shell_rc="$HOME/.bashrc"
    printf '\n# cargo-installed CLIs\nexport PATH="$HOME/.cargo/bin:$PATH"\n' >> "$shell_rc"
    echo "updated $shell_rc"
    ;;
esac
```

### source/Cargo update smoke

```sh
set -euo pipefail

repo_dir="${SOCAI_SOURCE_REPO:-$HOME/.socai/src/socai}"
before_path="$(command -v socai || true)"
before_version="$(socai --version 2>&1 || true)"
printf 'before_path=%s\nbefore_version=%s\n' "$before_path" "$before_version"

git -C "$repo_dir" fetch origin
git -C "$repo_dir" pull --ff-only
cd "$repo_dir"
git rev-parse HEAD
cargo install --path cli --force
install -m 0644 SKILL.md "$HOME/.socai/share/socai/SKILL.md"
install -m 0644 install.md "$HOME/.socai/share/socai/install.md"
socai stop || true

hash -r 2>/dev/null || true
command -v socai
socai --version
if socai --help | grep -q 'doctor'; then
  socai doctor
else
  echo 'doctor_missing_use_manual_install_md_checks'
fi
```

Expected:

- The repo updates cleanly with `--ff-only`.
- The binary remains at `$HOME/.cargo/bin/socai`.
- Any old daemon is stopped before validating the new CLI.

## WSL and native Windows notes

### WSL source/Cargo smoke

Treat WSL as Linux userland and use the source/Cargo checklist above.
Additional WSL-specific evidence:

```sh
set -euo pipefail

grep -i microsoft /proc/version || true
pwd
case "$PWD" in
  /mnt/*) echo 'warning: checkout is under /mnt; prefer WSL filesystem such as $HOME/.socai/src' ;;
  *) echo 'checkout_on_wsl_filesystem_or_non_mnt_path' ;;
esac
command -v socai || true
socai --version 2>&1 || true
```

Expected:

- The durable source checkout is under the WSL filesystem, not `/mnt/c`, unless
  the tester documents why that was unavoidable.
- Browser/CDP may point to Linux Chrome in WSL or Windows Chrome with a reachable
  debug port. Record which browser was used.

If using Windows Chrome from WSL, ask the user to start Chrome on Windows with a
remote-debugging port, then check endpoint reachability from WSL:

```sh
curl -fsS http://127.0.0.1:9222/json/version || true
export SOCAI_CDP_URL='http://127.0.0.1:9222'
```

### native Windows status

Do not mark native Windows as a supported managed-binary path for issue #71.
Current daemon IPC uses Unix-domain sockets, and native Windows porting is
tracked by issue #80. If a tester experiments on native Windows anyway, record it
as exploratory only:

```powershell
where.exe socai
socai --version
socai --help
```

Expected result for issue #71 today: native Windows remains documented as a
limitation unless issue #80 is completed and a native IPC path is verified.

## skill registration checks

After either install mode, verify canonical docs and agent registration. Use
paths appropriate for the tester's agents; mark unavailable agents as N/A with a
reason.

### canonical installed docs

```sh
set -euo pipefail

test -s "$HOME/.socai/share/socai/SKILL.md"
test -s "$HOME/.socai/share/socai/install.md"
ls -l "$HOME/.socai/share/socai/SKILL.md" "$HOME/.socai/share/socai/install.md"
```

If the archive or checkout source is available, compare exact content:

```sh
cmp -s SKILL.md "$HOME/.socai/share/socai/SKILL.md" && echo 'SKILL.md matches'
cmp -s install.md "$HOME/.socai/share/socai/install.md" && echo 'install.md matches'
```

### Claude Code registration

```sh
set -euo pipefail

mkdir -p "$HOME/.claude/skills/socai"
cp "$HOME/.socai/share/socai/SKILL.md" "$HOME/.claude/skills/socai/SKILL.md"
test -s "$HOME/.claude/skills/socai/SKILL.md"
cmp -s "$HOME/.socai/share/socai/SKILL.md" "$HOME/.claude/skills/socai/SKILL.md"
```

### Codex registration

```sh
set -euo pipefail

mkdir -p "$HOME/.codex/skills/socai"
cp "$HOME/.socai/share/socai/SKILL.md" "$HOME/.codex/skills/socai/SKILL.md"
mkdir -p "$HOME/.codex"
touch "$HOME/.codex/AGENTS.md"
if ! grep -Fq '~/.socai/share/socai/SKILL.md' "$HOME/.codex/AGENTS.md"; then
  cat >> "$HOME/.codex/AGENTS.md" <<'EOF'

## socai

When asked to use the socai CLI for Xiaohongshu research, read
`~/.socai/share/socai/SKILL.md` first and follow its stdout/stderr contract.
EOF
fi

test -s "$HOME/.codex/skills/socai/SKILL.md"
cmp -s "$HOME/.socai/share/socai/SKILL.md" "$HOME/.codex/skills/socai/SKILL.md"
grep -n 'socai' "$HOME/.codex/AGENTS.md"
```

Expected:

- The canonical installed `SKILL.md` exists.
- Claude Code and Codex copies match the canonical file when those agents are in
  scope.
- Existing unrelated agent instructions are not overwritten.

## browser/CDP troubleshooting checks

`socai` controls the user's browser through CDP. Browser login, manual
permission prompts, and CAPTCHA challenges are user actions; record when a user
had to intervene.

### passive CDP endpoint checks

```sh
set -euo pipefail

curl -fsS http://127.0.0.1:9222/json/version || true
curl -fsS http://127.0.0.1:9223/json/version || true
printf 'SOCAI_CDP_URL=%s\n' "${SOCAI_CDP_URL:-}"
printf 'SOCAI_CDP_WS=%s\n' "${SOCAI_CDP_WS:-}"
```

Expected:

- At least one endpoint responds before live browser automation, or the tester
  records how Chrome was started and which env var was set.

### start Chrome with remote debugging when needed

macOS:

```sh
/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
  --remote-debugging-port=9222 \
  --user-data-dir="$HOME/.socai/chrome-debug-profile"
```

Linux or WSL with a Linux GUI browser:

```sh
google-chrome \
  --remote-debugging-port=9222 \
  --user-data-dir="$HOME/.socai/chrome-debug-profile"
```

Then set one of:

```sh
export SOCAI_CDP_URL='http://127.0.0.1:9222'
# or, when using a browser websocket directly:
export SOCAI_CDP_WS='ws://127.0.0.1:9222/devtools/browser/<id>'
```

### command and daemon diagnostics

Use non-destructive CLI checks first:

```sh
socai --help
socai search_notes --help
socai topic_scan --help
socai extract_note --help
socai doctor
```

Only run live Xiaohongshu commands after the user agrees to use their logged-in
browser session. For a live smoke, keep the query small and record stdout and
stderr separately:

```sh
err_file="$(mktemp)"
json_file="$(mktemp)"
if socai search_notes '运营爆款思路' --pretty >"$json_file" 2>"$err_file"; then
  sed -n '1,80p' "$json_file"
  grep '^run_dir: ' "$err_file" || true
else
  cat "$err_file" >&2
  exit 1
fi
```

If CDP or browser automation fails, collect:

```sh
ls -la "$HOME/.socai" 2>/dev/null || true
tail -n 200 "$HOME/.socai/rust-daemon.log" 2>/dev/null || true
socai stop || true
if socai --help | grep -q 'doctor'; then socai doctor; fi
```

Expected:

- Help commands do not require browser state.
- `socai doctor` reports browser/CDP readiness or actionable failure details.
- Live commands keep JSON on stdout and metadata such as `run_dir:` on stderr.

## daemon version mismatch and stale-daemon checks

Run these checks after replacing a managed binary or reinstalling with Cargo.
They are especially important because an already-running daemon may still be the
old binary's code.

### manual stale-daemon recovery

```sh
set -euo pipefail

socai --version
socai stop || true
if socai --help | grep -q 'doctor'; then
  socai doctor
else
  echo 'doctor_missing_use_manual_install_md_checks'
fi
ls -la "$HOME/.socai" 2>/dev/null || true
tail -n 80 "$HOME/.socai/rust-daemon.log" 2>/dev/null || true
```

Expected:

- `socai stop || true` is safe even when no daemon is running.
- The next command starts or checks a daemon from the newly installed binary.

### version mismatch handshake smoke when available

If the build includes daemon version handshake behavior, capture an update from
old to new binary:

```text
old socai --version:
old daemon started by command:
new install method:
new socai --version:
first command after update without manual socai stop:
doctor or command output showing stale daemon detection/restart:
final socai stop output:
```

Expected for final #71 acceptance:

- A stale daemon is either restarted automatically by version handshake or
  clearly identified by `socai doctor` with a recovery step that an agent can
  follow.
- If handshake behavior is not present in the tested build, record the limitation
  and keep `socai stop || true` in the update runbook evidence.

## QA result template

Use this summary after completing each install mode.

```text
Mode:
Status: pass | fail | blocked | partial
Commands run:
Key output:
- command -v socai:
- socai --version:
- socai doctor:
- PATH/type -a:
- skill registration:
- CDP/browser checks:
- daemon stale-state checks:
What was not tested:
Limitations or follow-up issues:
```

## local validation captured while adding this runbook

This repository change did not perform a clean-machine managed-binary install.
The available GitHub Release did not contain the required CLI tarball at the
time of writing.

Command run:

```sh
gh release view --repo tonyc-ship/socai --json tagName,name,assets,publishedAt,url,isDraft,isPrerelease
```

Observed summary on 2026-06-06:

```text
tagName: v0.1.5
asset: socai-macos-universal.dmg
missing: socai-cli-macos-universal.tar.gz
missing: socai-cli-macos-universal.tar.gz.sha256
```

Therefore this PR is a QA runbook/status artifact for issue #71, not proof that
#71's clean macOS GitHub Release CLI tarball acceptance criterion has passed.
