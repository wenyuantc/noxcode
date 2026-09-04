import { create } from "zustand";

import {
  type AppUpdateInfo,
  type AppUpdateProgress,
  type UpdaterErrorCode,
  checkForAppUpdate,
  downloadAndInstallUpdate,
  isAppUpdateDevMode,
  mapUpdaterError,
  relaunchApp,
} from "@/lib/appUpdate";

export type AppUpdateStatus = "idle" | "available" | "downloading" | "ready";

export type SidebarUpdateLabelKey = "update" | "downloading" | "restartUpdate";

function errorDetail(cause: unknown): string {
  if (cause instanceof Error) {
    return cause.message;
  }
  return String(cause ?? "");
}

export function sidebarUpdateLabelKey(status: AppUpdateStatus): SidebarUpdateLabelKey | null {
  switch (status) {
    case "available":
      return "update";
    case "downloading":
      return "downloading";
    case "ready":
      return "restartUpdate";
    default:
      return null;
  }
}

interface UpdateState {
  status: AppUpdateStatus;
  checking: boolean;
  startupChecked: boolean;
  update: AppUpdateInfo | null;
  progress: AppUpdateProgress | null;
  errorCode: UpdaterErrorCode | null;
  errorDetail: string;
  relaunchFailedDetail: string | null;
  upToDate: boolean;
  checkOnStartup: () => Promise<void>;
  checkForUpdate: (options?: { silent?: boolean }) => Promise<void>;
  startDownload: () => Promise<void>;
  relaunch: () => Promise<void>;
  clearError: () => void;
}

export const useUpdateStore = create<UpdateState>((set, get) => ({
  status: "idle",
  checking: false,
  startupChecked: false,
  update: null,
  progress: null,
  errorCode: null,
  errorDetail: "",
  relaunchFailedDetail: null,
  upToDate: false,

  checkOnStartup: async () => {
    if (get().startupChecked) {
      return;
    }
    set({ startupChecked: true });
    if (isAppUpdateDevMode()) {
      return;
    }
    const { status } = get();
    if (status === "downloading" || status === "ready") {
      return;
    }
    await get().checkForUpdate({ silent: true });
  },

  checkForUpdate: async (options) => {
    const silent = options?.silent === true;
    const { status, checking } = get();
    if (checking || status === "downloading" || status === "ready") {
      return;
    }

    set({
      checking: true,
      errorCode: null,
      errorDetail: "",
      relaunchFailedDetail: null,
      upToDate: false,
    });

    try {
      const update = await checkForAppUpdate();
      if (!update) {
        set({
          checking: false,
          status: "idle",
          update: null,
          upToDate: true,
        });
        return;
      }
      set({
        checking: false,
        status: "available",
        update,
        upToDate: false,
      });
    } catch (cause) {
      if (silent) {
        set({ checking: false });
        return;
      }
      const code = mapUpdaterError(cause);
      set({
        checking: false,
        errorCode: code,
        errorDetail: errorDetail(cause),
        upToDate: code === "already_latest",
      });
    }
  },

  startDownload: async () => {
    const { update, status } = get();
    if (!update || status !== "available") {
      return;
    }

    set({
      status: "downloading",
      progress: { downloaded: 0, total: null, percent: null },
      errorCode: null,
      errorDetail: "",
      relaunchFailedDetail: null,
    });

    try {
      await downloadAndInstallUpdate(update, (progress) => {
        set({ progress });
      });
      set({ status: "ready" });
    } catch (cause) {
      set({
        status: "available",
        errorCode: mapUpdaterError(cause),
        errorDetail: errorDetail(cause),
      });
    }
  },

  relaunch: async () => {
    set({ relaunchFailedDetail: null, errorCode: null, errorDetail: "" });
    try {
      await relaunchApp();
    } catch (cause) {
      set({ relaunchFailedDetail: errorDetail(cause) });
    }
  },

  clearError: () =>
    set({
      errorCode: null,
      errorDetail: "",
      relaunchFailedDetail: null,
    }),
}));
