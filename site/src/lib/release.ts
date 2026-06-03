import { readFile } from "node:fs/promises";

// Resolves the release version shown in the nav chip / hero meta. Shared across
// every page so the displayed version is identical site-wide. Resolution order:
// explicit env override → local Tauri app config → latest GitHub release.

export function normalizeReleaseVersion(value: unknown): string | null {
    if (typeof value !== "string") {
        return null;
    }

    const match = value.trim().match(/^v?(\d+\.\d+\.\d+)$/);
    return match?.[1] ?? null;
}

async function fetchLatestReleaseVersion(): Promise<string | null> {
    try {
        const headers = new Headers({
            Accept: "application/vnd.github+json",
            "X-GitHub-Api-Version": "2022-11-28",
        });
        const token = import.meta.env.GITHUB_TOKEN || import.meta.env.GH_TOKEN;

        if (token) {
            headers.set("Authorization", `Bearer ${token}`);
        }

        const response = await fetch(
            "https://api.github.com/repos/tonyc-ship/socai/releases/latest",
            {
                headers,
            },
        );

        if (!response.ok) {
            return null;
        }

        const release = await response.json();
        return (
            normalizeReleaseVersion(release.tag_name) ??
            normalizeReleaseVersion(release.name)
        );
    } catch {
        return null;
    }
}

async function readLocalAppVersion(): Promise<string | null> {
    try {
        // site/src/lib/release.ts → ../../../ is the repo root, where the Tauri
        // app config lives. Same depth as the former site/src/pages/ caller.
        const configUrl = new URL(
            "../../../app/src-tauri/tauri.conf.json",
            import.meta.url,
        );
        const config = JSON.parse(await readFile(configUrl, "utf8"));
        return normalizeReleaseVersion(config.version);
    } catch {
        return null;
    }
}

export async function resolveReleaseVersion(): Promise<string> {
    const version =
        normalizeReleaseVersion(import.meta.env.SOCAI_RELEASE_VERSION) ??
        normalizeReleaseVersion(import.meta.env.PUBLIC_SOCAI_RELEASE_VERSION) ??
        (await readLocalAppVersion()) ??
        (await fetchLatestReleaseVersion());

    if (!version) {
        throw new Error(
            "Unable to resolve release version; set SOCAI_RELEASE_VERSION to a MAJOR.MINOR.PATCH value.",
        );
    }

    return version;
}
