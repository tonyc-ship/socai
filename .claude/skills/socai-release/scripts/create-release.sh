#!/usr/bin/env bash
set -euo pipefail

repo="socai-io/socai"
workflow="release.yml"
release_type="patch"
ref="main"
watch="true"

usage() {
  cat <<'USAGE'
Usage: create-release.sh [patch|minor|major] [options]

Trigger the socai GitHub Actions release workflow with gh, find the created
workflow run, optionally watch it to completion, and print release details.

Options:
  --type TYPE       Release bump: patch, minor, or major (default: patch)
  --ref REF         Git ref to dispatch from (default: main). Production
                    releases publish only from main. fix/release-* is for
                    non-publishing workflow tests.
  --repo OWNER/REPO GitHub repository (default: socai-io/socai)
  --workflow FILE   Workflow file/name (default: release.yml)
  --no-watch        Trigger and print run URL without waiting for completion
  -h, --help        Show this help

Examples:
  .claude/skills/socai-release/scripts/create-release.sh patch
  .claude/skills/socai-release/scripts/create-release.sh --type minor
  .claude/skills/socai-release/scripts/create-release.sh patch --ref fix/release-test
USAGE
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    patch|minor|major)
      release_type="$1"
      shift
      ;;
    --type)
      if [ "$#" -lt 2 ]; then
        echo "missing value for --type" >&2
        exit 2
      fi
      release_type="$2"
      shift 2
      ;;
    --ref)
      if [ "$#" -lt 2 ]; then
        echo "missing value for --ref" >&2
        exit 2
      fi
      ref="$2"
      shift 2
      ;;
    --repo)
      if [ "$#" -lt 2 ]; then
        echo "missing value for --repo" >&2
        exit 2
      fi
      repo="$2"
      shift 2
      ;;
    --workflow)
      if [ "$#" -lt 2 ]; then
        echo "missing value for --workflow" >&2
        exit 2
      fi
      workflow="$2"
      shift 2
      ;;
    --no-watch)
      watch="false"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

case "${release_type}" in
  patch|minor|major) ;;
  *)
    echo "release type must be patch, minor, or major; got: ${release_type}" >&2
    exit 2
    ;;
esac

case "${ref}" in
  main|fix/release-*) ;;
  *)
    echo "release workflow only permits main or fix/release-* refs; got: ${ref}" >&2
    exit 2
    ;;
esac

if ! command -v gh >/dev/null 2>&1; then
  echo "gh CLI is required: https://cli.github.com/" >&2
  exit 127
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required" >&2
  exit 127
fi

gh auth status --hostname github.com >/dev/null

echo "repo: ${repo}"
echo "workflow: ${workflow}"
echo "ref: ${ref}"
echo "release_type: ${release_type}"

if [ "${ref}" = "main" ]; then
  echo "production release: the workflow will publish a GitHub Release if the build succeeds"
else
  echo "test release run: publish job is skipped for ${ref}"
fi

started_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

gh workflow run "${workflow}" \
  --repo "${repo}" \
  --ref "${ref}" \
  -f "release_type=${release_type}"

echo "dispatched at: ${started_at}"
echo "waiting for workflow run to appear..."

run_id=""
run_url=""
for _ in $(seq 1 30); do
  runs_json="$(gh run list \
    --repo "${repo}" \
    --workflow "${workflow}" \
    --branch "${ref}" \
    --event workflow_dispatch \
    --limit 20 \
    --json databaseId,createdAt,status,conclusion,url)"

  run_record="$(printf '%s' "${runs_json}" | python3 -c '
import json
import sys
started = sys.argv[1]
runs = json.load(sys.stdin)
matches = [r for r in runs if r.get("createdAt", "") >= started]
if not matches:
    raise SystemExit(0)
matches.sort(key=lambda r: r.get("createdAt", ""), reverse=True)
r = matches[0]
print(str(r["databaseId"]) + "\t" + str(r["url"]))
' "${started_at}")"

  if [ -n "${run_record}" ]; then
    run_id="${run_record%%$'\t'*}"
    run_url="${run_record#*$'\t'}"
    break
  fi

  sleep 2
done

if [ -z "${run_id}" ]; then
  echo "could not find the dispatched run automatically" >&2
  echo "recent runs:" >&2
  gh run list --repo "${repo}" --workflow "${workflow}" --limit 10 >&2
  exit 1
fi

echo "run id: ${run_id}"
echo "run url: ${run_url}"

if [ "${watch}" != "true" ]; then
  exit 0
fi

gh run watch "${run_id}" --repo "${repo}" --exit-status

echo "run summary:"
gh run view "${run_id}" \
  --repo "${repo}" \
  --json databaseId,status,conclusion,url,createdAt,updatedAt,headBranch \
  --jq '{databaseId,status,conclusion,url,createdAt,updatedAt,headBranch}'

if [ "${ref}" = "main" ]; then
  echo "latest release:"
  gh release view \
    --repo "${repo}" \
    --json tagName,name,url,isDraft,isPrerelease,publishedAt,assets \
    --jq '{tagName,name,url,isDraft,isPrerelease,publishedAt,assets:[.assets[].name]}'
fi
