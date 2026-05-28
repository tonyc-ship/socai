import { readFile, writeFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = 'tonyc-ship/socai';
const assetName = 'socai-macos-universal.dmg';
const minimumMacos = 'macOS 13 or later';
const build = 'universal macOS build';
const downloadPath = '/download';
const downloadMacosPath = '/download/macos';
const githubUrl = `https://github.com/${repo}`;
const latestAssetUrl = `${githubUrl}/releases/latest/download/${assetName}`;
const apiUrl = `https://api.github.com/repos/${repo}/releases/latest`;
const githubToken = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;

const scriptDir = dirname(fileURLToPath(import.meta.url));
const releaseDataPath = resolve(scriptDir, '../src/data/release.json');

function formatFileSize(bytes) {
  const mb = bytes / 1_000_000;
  return `${mb.toFixed(mb >= 10 ? 1 : 2)} MB`;
}

function normalizeSha256(digest) {
  if (!digest) return '';
  return String(digest).replace(/^sha256:/, '');
}

async function readCurrentMetadata() {
  const raw = await readFile(releaseDataPath, 'utf8');
  return JSON.parse(raw);
}

async function fetchLatestMetadata(current) {
  const headers = {
    Accept: 'application/vnd.github+json',
    'User-Agent': 'socai-site-release-sync',
  };

  if (githubToken) {
    headers.Authorization = `Bearer ${githubToken}`;
  }

  const response = await fetch(apiUrl, { headers });

  if (!response.ok) {
    throw new Error(`GitHub Releases API returned ${response.status} ${response.statusText}`);
  }

  const release = await response.json();
  const asset = release.assets?.find((candidate) => candidate.name === assetName);

  if (!asset) {
    throw new Error(`release ${release.tag_name ?? '(unknown)'} does not contain ${assetName}`);
  }

  const tag = release.tag_name;

  if (!tag) {
    throw new Error('latest release is missing tag_name');
  }

  return {
    tag,
    version: tag.replace(/^v/, ''),
    assetName,
    build,
    fileSize: formatFileSize(asset.size),
    minimumMacos,
    downloadPath,
    downloadMacosPath,
    downloadUrl: latestAssetUrl,
    githubUrl,
    sha256: normalizeSha256(asset.digest) || current.sha256 || '',
  };
}

async function main() {
  if (process.env.SOCAI_SKIP_RELEASE_SYNC === '1') {
    console.warn('[release:sync] skipped because SOCAI_SKIP_RELEASE_SYNC=1');
    return;
  }

  const current = await readCurrentMetadata();

  try {
    const latest = await fetchLatestMetadata(current);
    const next = `${JSON.stringify(latest, null, 2)}\n`;
    const existing = `${JSON.stringify(current, null, 2)}\n`;

    if (next !== existing) {
      await writeFile(releaseDataPath, next);
      console.log(`[release:sync] updated release metadata to ${latest.tag}`);
    } else {
      console.log(`[release:sync] release metadata is current (${latest.tag})`);
    }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);

    if (process.env.SOCAI_RELEASE_SYNC_STRICT === '1') {
      throw new Error(`[release:sync] ${message}`);
    }

    console.warn(`[release:sync] ${message}; using checked-in release metadata (${current.tag})`);
  }
}

await main();
