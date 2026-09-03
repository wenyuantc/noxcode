import { create } from "zustand";

import {
  checkWorkspaceHealth,
  createWorkspace,
  deleteWorkspace,
  listAgentSessions,
  listWorkspaces,
  updateWorkspace,
} from "@/lib/backend";
import type { AgentSession, CreateWorkspaceInput, Workspace, WorkspaceHealth } from "@/lib/types";

const ACTIVE_KEY = "noxcode:active-workspace";

interface WorkspaceState {
  workspaces: Workspace[];
  sessions: AgentSession[];
  activeWorkspaceId: string | null;
  health: WorkspaceHealth | null;
  expanded: Record<string, boolean>;
  shownCount: Record<string, number>;
  loading: boolean;
  load: () => Promise<void>;
  setActive: (id: string | null) => Promise<void>;
  create: (payload: CreateWorkspaceInput) => Promise<Workspace>;
  rename: (id: string, name: string) => Promise<void>;
  remove: (id: string) => Promise<void>;
  toggleExpand: (id: string) => void;
  showMore: (id: string) => void;
  refreshSessions: () => Promise<void>;
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  workspaces: [],
  sessions: [],
  activeWorkspaceId: typeof window === "undefined" ? null : localStorage.getItem(ACTIVE_KEY),
  health: null,
  expanded: {},
  shownCount: {},
  loading: false,
  load: async () => {
    set({ loading: true });
    const [workspaces, sessions] = await Promise.all([listWorkspaces(), listAgentSessions()]);
    const stored = get().activeWorkspaceId;
    const active =
      stored && workspaces.some((item) => item.id === stored)
        ? stored
        : (workspaces[0]?.id ?? null);
    if (active) localStorage.setItem(ACTIVE_KEY, active);
    else localStorage.removeItem(ACTIVE_KEY);
    set({
      workspaces,
      sessions,
      activeWorkspaceId: active,
      loading: false,
      expanded: Object.fromEntries(workspaces.map((item) => [item.id, true])),
    });
    if (active) {
      const health = await checkWorkspaceHealth(active).catch(() => null);
      set({ health });
    }
  },
  setActive: async (id) => {
    if (id) localStorage.setItem(ACTIVE_KEY, id);
    else localStorage.removeItem(ACTIVE_KEY);
    set({ activeWorkspaceId: id, health: null });
    if (id) {
      const health = await checkWorkspaceHealth(id).catch(() => null);
      set({ health });
    }
  },
  create: async (payload) => {
    const workspace = await createWorkspace(payload);
    await get().load();
    await get().setActive(workspace.id);
    return workspace;
  },
  rename: async (id, name) => {
    await updateWorkspace(id, { name });
    await get().load();
  },
  remove: async (id) => {
    await deleteWorkspace(id);
    await get().load();
  },
  toggleExpand: (id) => set({ expanded: { ...get().expanded, [id]: !get().expanded[id] } }),
  showMore: (id) =>
    set({ shownCount: { ...get().shownCount, [id]: (get().shownCount[id] ?? 5) + 10 } }),
  refreshSessions: async () => {
    const sessions = await listAgentSessions();
    set({ sessions });
  },
}));
