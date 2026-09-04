import { getVersion } from "@tauri-apps/api/app";
import { isTauri } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { check, type DownloadEvent, type Update } from "@tauri-apps/plugin-updater";

export type UpdaterErrorCode =
  "network" | "already_latest" | "signature" | "dev_mode" | "cancelled" | "unknown";

export type AppUpdateInfo = {
  version: string;
  currentVersion: string;
  notes: string | null;
  pubDate: string | null;
};

export type AppUpdateProgress = {
  downloaded: number;
  total: number | null;
  percent: number | null;
};

const pendingUpdates = new WeakMap<AppUpdateInfo, Update>();

function errorText(error: unknown): string {
  if (error == null) {
    return "";
  }
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return `${error.name} ${error.message}`.trim();
  }
  if (typeof error === "object") {
    const candidate = error as { message?: unknown; error?: unknown; kind?: unknown };
    return [candidate.message, candidate.error, candidate.kind]
      .filter((value): value is string => typeof value === "string")
      .join(" ");
  }
  return String(error);
}

export function mapUpdaterError(error: unknown): UpdaterErrorCode {
  if (error == null) {
    return "already_latest";
  }

  const text = errorText(error).toLowerCase().trim();
  if (!text) {
    return typeof error === "string" ? "already_latest" : "unknown";
  }

  if (/(?:^|[^a-z])(?:cancel(?:led|ed)?|aborted)(?:$|[^a-z])/.test(text)) {
    return "cancelled";
  }

  if (
    /development mode|dev mode|tauri dev|updates? are (?:not available|disabled)|cannot check for (?:an )?updates? in/.test(
      text,
    )
  ) {
    return "dev_mode";
  }

  if (
    /signature|minisign|public key|pubkey|failed to verify|invalid key|could not be decoded/.test(
      text,
    )
  ) {
    return "signature";
  }

  if (/already (?:up to date|latest)|up to date|no updates? available/.test(text)) {
    return "already_latest";
  }

  if (
    /could not fetch a valid release json|network|fetch|connect|connection|dns|offline|timed? ?out|error sending request|failed to send|econnrefused|enotfound|unreachable|reqwest/.test(
      text,
    )
  ) {
    return "network";
  }

  return "unknown";
}

export function updaterErrorI18nKey(code: UpdaterErrorCode): string {
  switch (code) {
    case "already_latest":
      return "about.errors.alreadyLatest";
    case "dev_mode":
      return "about.errors.devMode";
    default:
      return `about.errors.${code}`;
  }
}

export function isAppUpdateDevMode(): boolean {
  return import.meta.env.DEV === true || !isTauri();
}

export async function getAppVersion(): Promise<string> {
  return getVersion();
}

export async function checkForAppUpdate(): Promise<AppUpdateInfo | null> {
  if (isAppUpdateDevMode()) {
    throw new Error("Updates are not available in development mode");
  }

  const update = await check();
  if (!update) {
    return null;
  }

  const info: AppUpdateInfo = {
    version: update.version,
    currentVersion: update.currentVersion,
    notes: update.body?.trim() ? update.body.trim() : null,
    pubDate: update.date ?? null,
  };
  pendingUpdates.set(info, update);
  return info;
}

export async function downloadAndInstallUpdate(
  update: AppUpdateInfo,
  onProgress?: (progress: AppUpdateProgress) => void,
): Promise<void> {
  const handle = pendingUpdates.get(update);
  if (!handle) {
    throw new Error("No pending update is available to install");
  }

  let downloaded = 0;
  let total: number | null = null;

  const emitProgress = () => {
    onProgress?.({
      downloaded,
      total,
      percent: total && total > 0 ? Math.min(100, Math.round((downloaded / total) * 100)) : null,
    });
  };

  await handle.downloadAndInstall((event: DownloadEvent) => {
    if (event.event === "Started") {
      downloaded = 0;
      total = event.data.contentLength ?? null;
      emitProgress();
      return;
    }
    if (event.event === "Progress") {
      downloaded += event.data.chunkLength;
      emitProgress();
      return;
    }
    if (event.event === "Finished") {
      if (total != null) {
        downloaded = total;
      }
      emitProgress();
    }
  });
}

export async function relaunchApp(): Promise<void> {
  await relaunch();
}
