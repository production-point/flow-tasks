import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

interface GitHubRelease {
  tag_name: string;
  html_url: string;
  assets: Array<{ name: string; browser_download_url: string }>;
}

export async function checkForUpdate(currentVersion: string): Promise<{
  available: boolean;
  version?: string;
  downloadUrl?: string;
} | null> {
  try {
    const res = await tauriFetch(
      "https://api.github.com/repos/production-point/flow-tasks/releases/latest",
      {
        headers: { Accept: "application/vnd.github.v3+json" },
      }
    );
    if (!res.ok) return null;
    const release: GitHubRelease = await res.json();
    const latestVersion = release.tag_name.replace(/^v/, "");
    if (compareVersions(latestVersion, currentVersion) > 0) {
      const installer = release.assets.find((a) => a.name.endsWith("-setup.exe"));
      return {
        available: true,
        version: latestVersion,
        downloadUrl: installer?.browser_download_url || release.html_url,
      };
    }
    return { available: false };
  } catch {
    return null;
  }
}

function compareVersions(a: string, b: string): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if ((pa[i] || 0) > (pb[i] || 0)) return 1;
    if ((pa[i] || 0) < (pb[i] || 0)) return -1;
  }
  return 0;
}
