import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AppUpdateInfo } from "@/lib/appUpdate";

vi.mock("@/lib/appUpdate", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/appUpdate")>();
  return {
    ...actual,
    checkForAppUpdate: vi.fn(),
    downloadAndInstallUpdate: vi.fn(),
    isAppUpdateDevMode: vi.fn(() => false),
    relaunchApp: vi.fn(),
  };
});

import {
  checkForAppUpdate,
  downloadAndInstallUpdate,
  isAppUpdateDevMode,
  relaunchApp,
} from "@/lib/appUpdate";
import { sidebarUpdateLabelKey, useUpdateStore } from "./updateStore";

const check = vi.mocked(checkForAppUpdate);
const download = vi.mocked(downloadAndInstallUpdate);
const isDev = vi.mocked(isAppUpdateDevMode);
const relaunch = vi.mocked(relaunchApp);

const sampleUpdate: AppUpdateInfo = {
  version: "0.3.0",
  currentVersion: "0.2.3",
  notes: "fix",
  pubDate: "2026-09-04",
};

function resetStore() {
  useUpdateStore.setState({
    status: "idle",
    checking: false,
    startupChecked: false,
    update: null,
    progress: null,
    errorCode: null,
    errorDetail: "",
    relaunchFailedDetail: null,
    upToDate: false,
  });
}

describe("sidebarUpdateLabelKey", () => {
  it("maps the three sidebar button states", () => {
    expect(sidebarUpdateLabelKey("idle")).toBeNull();
    expect(sidebarUpdateLabelKey("available")).toBe("update");
    expect(sidebarUpdateLabelKey("downloading")).toBe("downloading");
    expect(sidebarUpdateLabelKey("ready")).toBe("restartUpdate");
  });
});

describe("updateStore", () => {
  beforeEach(() => {
    resetStore();
    check.mockReset();
    download.mockReset();
    relaunch.mockReset();
    isDev.mockReset();
    isDev.mockReturnValue(false);
  });

  it("silently records an available update on startup", async () => {
    check.mockResolvedValue(sampleUpdate);
    await useUpdateStore.getState().checkOnStartup();
    expect(useUpdateStore.getState()).toMatchObject({
      startupChecked: true,
      status: "available",
      update: sampleUpdate,
      upToDate: false,
    });
  });

  it("does not check again after the first startup pass", async () => {
    check.mockResolvedValue(sampleUpdate);
    await useUpdateStore.getState().checkOnStartup();
    check.mockClear();
    await useUpdateStore.getState().checkOnStartup();
    expect(check).not.toHaveBeenCalled();
  });

  it("skips the startup check in development mode", async () => {
    isDev.mockReturnValue(true);
    await useUpdateStore.getState().checkOnStartup();
    expect(check).not.toHaveBeenCalled();
    expect(useUpdateStore.getState().startupChecked).toBe(true);
  });

  it("swallows startup errors without exposing them", async () => {
    check.mockRejectedValue(new Error("error sending request"));
    await useUpdateStore.getState().checkOnStartup();
    expect(useUpdateStore.getState()).toMatchObject({
      status: "idle",
      errorCode: null,
      update: null,
    });
  });

  it("marks the app up to date when no release is available", async () => {
    check.mockResolvedValue(null);
    await useUpdateStore.getState().checkForUpdate();
    expect(useUpdateStore.getState()).toMatchObject({
      status: "idle",
      update: null,
      upToDate: true,
      checking: false,
    });
  });

  it("surfaces a visible error when a manual check fails", async () => {
    check.mockRejectedValue(new Error("Updates are not available in development mode"));
    await useUpdateStore.getState().checkForUpdate();
    expect(useUpdateStore.getState()).toMatchObject({
      errorCode: "dev_mode",
      checking: false,
    });
  });

  it("keeps the previous update visible while a later check is in flight", async () => {
    let resolveCheck: ((value: AppUpdateInfo | null) => void) | undefined;
    check.mockReturnValue(
      new Promise((resolve) => {
        resolveCheck = resolve;
      }),
    );
    useUpdateStore.setState({ status: "available", update: sampleUpdate });
    const pending = useUpdateStore.getState().checkForUpdate();
    expect(useUpdateStore.getState()).toMatchObject({
      checking: true,
      status: "available",
      update: sampleUpdate,
    });
    resolveCheck?.(sampleUpdate);
    await pending;
    expect(useUpdateStore.getState().checking).toBe(false);
  });

  it("does not start another check while downloading or ready to restart", async () => {
    useUpdateStore.setState({ status: "downloading", update: sampleUpdate });
    await useUpdateStore.getState().checkForUpdate();
    expect(check).not.toHaveBeenCalled();

    useUpdateStore.setState({ status: "ready", update: sampleUpdate });
    await useUpdateStore.getState().checkForUpdate();
    expect(check).not.toHaveBeenCalled();
  });

  it("downloads then marks the update ready to relaunch", async () => {
    download.mockImplementation(async (_update, onProgress) => {
      onProgress?.({ downloaded: 5, total: 10, percent: 50 });
    });
    useUpdateStore.setState({ status: "available", update: sampleUpdate });
    const pending = useUpdateStore.getState().startDownload();
    expect(useUpdateStore.getState().status).toBe("downloading");
    await pending;
    expect(useUpdateStore.getState()).toMatchObject({
      status: "ready",
      progress: { downloaded: 5, total: 10, percent: 50 },
    });
  });

  it("returns to the update button if download fails", async () => {
    download.mockRejectedValue(new Error("minisign: invalid signature"));
    useUpdateStore.setState({ status: "available", update: sampleUpdate });
    await useUpdateStore.getState().startDownload();
    expect(useUpdateStore.getState()).toMatchObject({
      status: "available",
      errorCode: "signature",
    });
  });

  it("records relaunch failures without changing status", async () => {
    relaunch.mockRejectedValue(new Error("boom"));
    useUpdateStore.setState({ status: "ready", update: sampleUpdate });
    await useUpdateStore.getState().relaunch();
    expect(useUpdateStore.getState()).toMatchObject({
      status: "ready",
      relaunchFailedDetail: "boom",
    });
  });
});
