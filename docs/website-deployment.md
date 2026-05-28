# Website deployment

The Socai marketing/download site lives in [`site/`](../site/) and is deployed to Vercel.

## Vercel project

- Team/scope: `socai-d83824c8` (`socai`)
- Project: `socai-site`
- Production domain: `https://socai.io`
- `www` behavior: `https://www.socai.io/*` redirects permanently to `https://socai.io/*`

Project settings:

| Setting | Value |
| --- | --- |
| Framework preset | Astro |
| Root directory | `site` |
| Install command | `pnpm install` |
| Build command | `pnpm build` |
| Output directory | `dist` |
| Node.js version | 24.x |

The project uses [`site/vercel.json`](../site/vercel.json) for download/GitHub redirects and the `www` canonical-host redirect.

## Domains

Both domains are attached to the `socai-site` Vercel project:

- `socai.io`
- `www.socai.io`

`socai.io` is registered at Vercel and uses Vercel nameservers, so no external DNS changes are required.

## Local validation

```bash
cd site
pnpm install
pnpm build
```

## Manual production deploy

If you need to deploy from a local checkout:

```bash
cd site
vercel link --yes --scope socai-d83824c8 --project socai-site
vercel deploy --prod --scope socai-d83824c8
```

After deployment, verify:

```bash
curl -I https://socai.io/
curl -I https://www.socai.io/
curl -I https://socai.io/download
curl -I https://socai.io/github
```

Expected behavior:

- `https://socai.io/` serves the static website.
- `https://www.socai.io/` redirects to `https://socai.io/`.
- `https://socai.io/download` redirects to the latest universal macOS DMG on GitHub Releases.
- `https://socai.io/github` redirects to `https://github.com/tonyc-ship/socai`.

## Git previews

Vercel preview deployments for pull requests require the `socai-site` project to be connected to the GitHub repository `tonyc-ship/socai` in Vercel.

If `vercel git connect https://github.com/tonyc-ship/socai --scope socai-d83824c8` reports that a GitHub login connection is missing, open the Vercel dashboard and add/connect the GitHub account or installation first, then connect the project to the repository with root directory `site`.
