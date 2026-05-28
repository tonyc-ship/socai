---
name: socai-release
description: Create, monitor, troubleshoot, and verify socai desktop GitHub releases from the command line with gh. Use when publishing a new release, triggering .github/workflows/release.yml, choosing patch/minor/major bumps, checking release workflow runs, or validating the latest macOS DMG/download redirect without using the GitHub UI.
---

# socai release

Use this skill whenever the task is to create or inspect a socai GitHub Release, trigger the release workflow, publish a new macOS DMG, or avoid clicking through the GitHub Actions / Releases UI.

The command-line release path is the source of truth:

```bash
gh workflow run release.yml --repo tonyc-ship/socai --ref main -f release_type=patch
```

A helper script wraps this command, finds the created run, watches it, and prints the resulting release:

```bash
.claude/skills/socai-release/scripts/create-release.sh patch
```

## What the release workflow does

- Workflow: `.github/workflows/release.yml`
- Trigger: manual `workflow_dispatch`
- Required input: `release_type` = `patch`, `minor`, or `major`
- Production ref: `main`
- Test ref: `fix/release-*` branches only; build runs but publish job is skipped
- Version source: latest strict semver tag matching `vMAJOR.MINOR.PATCH`; if no tag exists, app version from `app/src-tauri/tauri.conf.json`
- Artifact: `socai-macos-universal.dmg`
- Production publish steps on `main`:
  1. Build universal macOS app + DMG on GitHub Actions.
  2. Require Developer ID signing and notarization secrets.
  3. Update app version files with `.github/scripts/set-app-version.py`.
  4. Commit `chore: release socai vX.Y.Z` to `main` if needed.
  5. Tag the release commit as `vX.Y.Z`.
  6. Create a draft GitHub Release with generated notes and the DMG.
  7. Push updated `main`.
  8. Publish the release by clearing draft status.

## Safety rules

- Do not use the GitHub web UI for routine releases; use `gh`.
- Do not manually create GitHub releases or upload DMGs unless the workflow is broken and the user explicitly approves a fallback.
- Ask/confirm the release bump before production publishing if the user did not specify `patch`, `minor`, or `major`.
- Production releases must be dispatched from `main`; the workflow rejects other refs except `fix/release-*` test branches.
- Do not cancel an in-progress production release unless the user explicitly asks.
- Do not delete tags/releases unless the workflow failed and the user explicitly approves cleanup.
- If local files are dirty, do not include unrelated changes in release work. The workflow publishes from the remote ref, not local uncommitted files.

## Preflight checks

Run from the repo root when possible.

```bash
gh auth status --hostname github.com
gh repo view tonyc-ship/socai --json nameWithOwner,defaultBranchRef,url

git fetch origin main --tags
git status --short
git log --oneline --decorate -5 origin/main
```

Optional: preview the next version locally using the same bump semantics as the workflow:

```bash
release_type=patch  # patch | minor | major
latest_tag="$(git tag --list 'v*' --sort=-v:refname | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | head -n 1 || true)"
base_version="${latest_tag#v}"
if [ -z "${base_version}" ]; then
  base_version="$(python3 - <<'PY'
import json
from pathlib import Path
print(json.loads(Path('app/src-tauri/tauri.conf.json').read_text())['version'])
PY
)"
fi
python3 - "${base_version}" "${release_type}" <<'PY'
import sys
base, bump = sys.argv[1:]
major, minor, patch = map(int, base.split('.'))
if bump == 'major':
    major, minor, patch = major + 1, 0, 0
elif bump == 'minor':
    minor, patch = minor + 1, 0
elif bump == 'patch':
    patch += 1
else:
    raise SystemExit(f'bad release_type: {bump}')
print(f'v{major}.{minor}.{patch}')
PY
```

## Create a production release

Preferred helper:

```bash
.claude/skills/socai-release/scripts/create-release.sh patch
```

Equivalent raw `gh` path:

```bash
gh workflow run release.yml \
  --repo tonyc-ship/socai \
  --ref main \
  -f release_type=patch

sleep 8
run_id="$(gh run list \
  --repo tonyc-ship/socai \
  --workflow release.yml \
  --branch main \
  --event workflow_dispatch \
  --limit 1 \
  --json databaseId \
  --jq '.[0].databaseId')"

gh run watch "${run_id}" --repo tonyc-ship/socai --exit-status
```

Use `minor` or `major` instead of `patch` only when requested.

## Test the release workflow without publishing

Only use a branch named `fix/release-*`. The workflow will build, use ad-hoc signing if production signing secrets are unavailable, and skip the publish job.

```bash
.claude/skills/socai-release/scripts/create-release.sh patch --ref fix/release-some-branch
```

or:

```bash
gh workflow run release.yml \
  --repo tonyc-ship/socai \
  --ref fix/release-some-branch \
  -f release_type=patch
```

## Monitor or troubleshoot an existing run

List recent release runs:

```bash
gh run list --repo tonyc-ship/socai --workflow release.yml --limit 10
```

Watch a run:

```bash
gh run watch RUN_ID --repo tonyc-ship/socai --exit-status
```

View details/logs:

```bash
gh run view RUN_ID --repo tonyc-ship/socai
gh run view RUN_ID --repo tonyc-ship/socai --log-failed
```

Common failure notes:

- `main moved while the release was building`: rerun from the latest `main` after confirming the move was expected.
- Missing Apple secrets on `main`: production release cannot proceed until signing/notarization secrets are configured.
- Build/notarization failure after no `main` push: inspect logs; the workflow attempts to clean up draft release/tag state.
- Failure after `main was already updated`: leave state for manual inspection; do not delete the release/tag without explicit approval.

## Verify the published release

After a successful production run:

```bash
gh release view --repo tonyc-ship/socai \
  --json tagName,name,url,isDraft,isPrerelease,publishedAt,assets \
  --jq '{tagName,name,url,isDraft,isPrerelease,publishedAt,assets:[.assets[].name]}'
```

Expected:

- `isDraft: false`
- `isPrerelease: false`
- Asset list includes `socai-macos-universal.dmg`

Verify download redirects:

```bash
curl -I https://socai.io/download
curl -I -L --max-time 30 -o /dev/null -w 'code=%{http_code}\nfinal=%{url_effective}\n' https://socai.io/download
```

Expected final URL should resolve through GitHub latest release download and produce a successful response.

Optional artifact check:

```bash
tag="$(gh release view --repo tonyc-ship/socai --json tagName --jq '.tagName')"
mkdir -p "/tmp/socai-release-${tag}"
gh release download "${tag}" \
  --repo tonyc-ship/socai \
  --pattern socai-macos-universal.dmg \
  --dir "/tmp/socai-release-${tag}" \
  --clobber
shasum -a 256 "/tmp/socai-release-${tag}/socai-macos-universal.dmg"
```

## Reporting back

Include:

- Release type (`patch`, `minor`, or `major`)
- Ref used (`main` for production)
- GitHub Actions run URL and conclusion
- Published tag/version and release URL
- Asset presence (`socai-macos-universal.dmg`)
- `/download` verification summary
- Any failures, cleanup performed, or manual blockers
