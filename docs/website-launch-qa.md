# Website launch QA

Date: 2026-05-28

Scope: final launch checklist for [`https://socai.io`](https://socai.io), issue #34.

## Summary

Status: **website launch checks passed on production**, with one release-artifact follow-up documented.

- Website HTTPS, canonical host, download redirect, GitHub redirect, robots, sitemap, and social image checks passed.
- Latest GitHub release metadata matches the website download target.
- The latest DMG was downloaded through `https://socai.io/download`; its SHA-256 matches the GitHub Release digest.
- The mounted `socai.app` is code-signed and accepted as a notarized Developer ID app.
- The DMG container itself is **not notarized**; tracked separately in [#40](https://github.com/socai-io/socai/issues/40).
- A Lighthouse accessibility contrast issue was found in the app mock window title, fixed, deployed, and re-tested.
- Vercel Git preview deployments remain blocked until a Vercel/GitHub account owner connects the GitHub login/integration, as documented in [#33](https://github.com/socai-io/socai/issues/33).

## Build and config validation

Commands:

```bash
python3 -m json.tool site/vercel.json >/dev/null
python3 -m json.tool site/public/site.webmanifest >/dev/null
python3 - <<'PY'
import xml.etree.ElementTree as ET
ET.parse('site/public/sitemap.xml')
PY
cd site && pnpm build
```

Result: passed.

Vercel project settings confirmed with:

```bash
vercel project inspect socai-site --scope socai-d83824c8 --yes
```

Expected settings were present:

- Framework preset: Astro
- Root directory: `site`
- Install command: `pnpm install`
- Build command: `pnpm build`
- Output directory: `dist`
- Node.js version: 24.x

## Production URL checks

Commands:

```bash
curl -I https://socai.io/
curl -I https://www.socai.io/
curl -I https://socai.io/download
curl -I https://socai.io/download/macos
curl -I https://socai.io/github
curl -I https://socai.io/robots.txt
curl -I https://socai.io/sitemap.xml
curl -I https://socai.io/social-card.png
```

Results:

| URL | Expected | Result |
| --- | --- | --- |
| `https://socai.io/` | HTTPS 200 | Passed; `HTTP/2 200` |
| `https://www.socai.io/` | Redirect to canonical host | Passed; `HTTP/2 308`, `location: https://socai.io/` |
| `https://socai.io/download` | Redirect to latest universal DMG | Passed; `HTTP/2 307`, GitHub latest DMG location |
| `https://socai.io/download/macos` | Redirect to latest universal DMG | Passed; `HTTP/2 307`, GitHub latest DMG location |
| `https://socai.io/github` | Redirect to repo | Passed; `HTTP/2 307`, `location: https://github.com/socai-io/socai` |
| `https://socai.io/robots.txt` | HTTPS 200 | Passed |
| `https://socai.io/sitemap.xml` | HTTPS 200 | Passed |
| `https://socai.io/social-card.png` | HTTPS 200 image | Passed |

## Release metadata and download artifact

GitHub latest release checked with:

```bash
gh release view --repo socai-io/socai --json tagName,name,url,assets,publishedAt,isDraft,isPrerelease
```

Current latest release:

- Tag: `v0.1.2`
- Published: `2026-05-27T09:19:10Z`
- Asset: `socai-macos-universal.dmg`
- Size: `14,575,965` bytes (`14.6 MB` decimal)
- Release asset URL: `https://github.com/socai-io/socai/releases/download/v0.1.2/socai-macos-universal.dmg`
- Digest: `sha256:507ff343cc13b3453346d9d3663bb5e12bbda47507ed7e851b0bf65cfc893309`

Downloaded through the production website route:

```bash
curl -L --fail -o /tmp/socai-launch-qa/socai-macos-universal.dmg https://socai.io/download
shasum -a 256 /tmp/socai-launch-qa/socai-macos-universal.dmg
```

Result:

```text
507ff343cc13b3453346d9d3663bb5e12bbda47507ed7e851b0bf65cfc893309
```

The downloaded file hash matches the GitHub Release digest.

## Signing and notarization

DMG container check:

```bash
spctl -a -vv -t open --context context:primary-signature /tmp/socai-launch-qa/socai-macos-universal.dmg
```

Result:

```text
/tmp/socai-launch-qa/socai-macos-universal.dmg: rejected
source=Unnotarized Developer ID
origin=Developer ID Application: Mingrui Zhang (J9B2NB3X6G)
```

Mounted app checks:

```bash
hdiutil attach /tmp/socai-launch-qa/socai-macos-universal.dmg -nobrowse -readonly
codesign --verify --deep --strict --verbose=4 /Volumes/.../socai.app
spctl -a -vv -t exec /Volumes/.../socai.app
```

Results:

```text
socai.app: valid on disk
socai.app: satisfies its Designated Requirement
socai.app: accepted
source=Notarized Developer ID
origin=Developer ID Application: Mingrui Zhang (J9B2NB3X6G)
```

Conclusion: the app bundle is signed/notarized, but the DMG container should be notarized/stapled before broader public sharing. Follow-up: [#40](https://github.com/socai-io/socai/issues/40).

## Desktop and mobile browser checks

Headless Chrome screenshots were captured locally for:

- Desktop viewport: `1440 x 1200`
- Mobile viewport: `390 x 1100`

The screenshots rendered the landing page successfully. Chrome produced updater/background-process logs and required timeout cleanup, but screenshot files were written successfully.

## Performance and accessibility

Live Lighthouse before fixes:

- Performance: 86
- Accessibility: 95
- Best Practices: 100
- SEO: 100

Findings and fixes:

- Accessibility: low-contrast app mock window title. Fixed by changing `.window-title` from `--fg-soft` to `--fg-muted`.
- Best Practices: browser-side GitHub Releases API lookup could hit API rate limits and log a 403 console error. Removed the browser API call and kept non-stale generic `latest release` copy; `/download` still tracks GitHub latest.

Production Lighthouse after fixes and redeploy:

- Performance: 82
- Accessibility: 100
- Best Practices: 100
- SEO: 100

Remaining non-blocking performance notes are mostly related to external font loading/cache lifetime and render-blocking font CSS. Existing issue [#8](https://github.com/socai-io/socai/issues/8) already tracks bundling fonts locally.

## Rollback and update process

Confirmed update path:

1. Change site code under `site/`.
2. Build locally with `cd site && pnpm build`.
3. Deploy through Vercel project `socai-site`.
4. Verify production with the checks above.

Deployment runbook lives in the shared skill:

- [`.claude/skills/socai-site-deployment/SKILL.md`](../.claude/skills/socai-site-deployment/SKILL.md)

Known deployment process limitation: Vercel Git preview deployments are blocked until the GitHub login/integration is connected in Vercel. Manual production deployment is documented in the skill.

## Final checklist

- [x] `socai.io` loads over HTTPS.
- [x] `www.socai.io` redirects to `socai.io`.
- [x] `/download` resolves to latest universal macOS DMG.
- [x] `/download/macos` resolves to latest universal macOS DMG.
- [x] `/github` redirects to this repo.
- [x] Latest release metadata/version/file size checked.
- [x] Downloaded DMG SHA-256 matches GitHub Release digest.
- [x] App bundle signing/notarization checked.
- [x] DMG container notarization gap documented in #40.
- [x] Desktop and mobile headless browser screenshots captured.
- [x] Basic performance/accessibility checked with Lighthouse.
- [x] Browser-side GitHub API metadata fetch removed to avoid rate-limit console errors.
- [x] Accessibility contrast issue fixed and redeployed.
- [x] Rollback/update process confirmed and documented in the deployment skill.
