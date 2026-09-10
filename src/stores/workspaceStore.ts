import { create } from "zustand";

import {
  checkWorkspaceHealth,
  createWorkspace,
  deleteWorkspace,
  listAgentSessions,
  listWorkspaces,
  renameAgentSession,
  setAgentSessionArchived,
  setAgentSessionPinned,
  updateWorkspace,
} from "@/lib/backend";
import { mergeSessions } from "@/lib/sessionActions";
import type { AgentSession, CreateWorkspaceInput, Workspace, WorkspaceHealth } from "@/lib/types";
import { useSessionStore } from "@/stores/sessionStore";

const ACTIVE_KEY = "noxcode:active-workspace";
const EXPANDED_KEY = "noxcode:workspace-expanded";
const ARCHIVE_PAGE_SIZE = 50;
let sessionRevision = 0;
let sessionRequest = 0;
let archiveRequest = 0;

interface WorkspaceState {
  workspaces: Workspace[];
  sessions: AgentSession[];
  archivedSessionIds: string[];
  archivedLoaded: boolean;
  archivedLoading: boolean;
  archivedHasMore: boolean;
  archivedError: string | null;
  sessionMutations: Record<string, boolean>;
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
  loadArchivedSessions: (more?: boolean) => Promise<void>;
  renameSession: (id: string, title: string) => Promise<void>;
  setSessionPinned: (id: string, pinned: boolean) => Promise<void>;
  setSessionArchived: (id: string, archived: boolean) => Promise<void>;
}

function readExpanded(): Record<string, boolean> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(EXPANDED_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    const result: Record<string, boolean> = {};
    for (const [id, value] of Object.entries(parsed)) {
      if (typeof value === "boolean") result[id] = value;
    }
    return result;
  } catch {
    return {};
  }
}

function persistExpanded(expanded: Record<string, boolean>) {
  if (typeof window === "undefined") return;
  window.localStorage.setItem(EXPANDED_KEY, JSON.stringify(expanded));
}

function mergeExpanded(
  workspaces: Workspace[],
  stored: Record<string, boolean>,
): Record<string, boolean> {
  return Object.fromEntries(workspaces.map((item) => [item.id, stored[item.id] !== false]));
}

async function mutateSession(id: string, operation: () => Promise<AgentSession | void>) {
  if (useWorkspaceStore.getState().sessionMutations[id]) return false;
  sessionRevision += 1;
  useWorkspaceStore.setState((state) => ({
    sessionMutations: { ...state.sessionMutations, [id]: true },
  }));
  try {
    const session = await operation();
    if (session) {
      useWorkspaceStore.setState((state) => ({
        sessions: mergeSessions(state.sessions, [session]),
        archivedSessionIds: session.archived
          ? state.archivedSessionIds
          : state.archivedSessionIds.filter((sessionId) => sessionId !== session.id),
      }));
    }
    return true;
  } finally {
    sessionRevision += 1;
    useWorkspaceStore.setState((state) => {
      const sessionMutations = { ...state.sessionMutations };
      delete sessionMutations[id];
      return { sessionMutations };
    });
    const state = useWorkspaceStore.getState();
    void state.refreshSessions().catch(() => {});
    if (state.archivedLoaded || state.archivedLoading) void state.loadArchivedSessions();
  }
}

export const useWorkspaceStore = create<WorkspaceState>((set, get) => ({
  workspaces: [],
  sessions: [],
  archivedSessionIds: [],
  archivedLoaded: false,
  archivedLoading: false,
  archivedHasMore: false,
  archivedError: null,
  sessionMutations: {},
  activeWorkspaceId: typeof window === "undefined" ? null : localStorage.getItem(ACTIVE_KEY),
  health: null,
  expanded: readExpanded(),
  shownCount: {},
  loading: false,
  load: async () => {
    set({ loading: true });
    const workspaces = await listWorkspaces();
    const stored = get().activeWorkspaceId;
    const active =
      stored && workspaces.some((item) => item.id === stored)
        ? stored
        : (workspaces[0]?.id ?? null);
    if (active) localStorage.setItem(ACTIVE_KEY, active);
    else localStorage.removeItem(ACTIVE_KEY);
    const expanded = mergeExpanded(workspaces, { ...readExpanded(), ...get().expanded });
    persistExpanded(expanded);
    set({
      workspaces,
      sessions: get().sessions.filter(
        (session) =>
          !session.workspace_id || workspaces.some((item) => item.id === session.workspace_id),
      ),
      activeWorkspaceId: active,
      loading: false,
      expanded,
    });
    await get().refreshSessions();
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
  toggleExpand: (id) => {
    const next = { ...get().expanded, [id]: !(get().expanded[id] !== false) };
    persistExpanded(next);
    set({ expanded: next });
  },
  showMore: (id) =>
    set({ shownCount: { ...get().shownCount, [id]: (get().shownCount[id] ?? 5) + 10 } }),
  refreshSessions: async () => {
    const request = ++sessionRequest;
    const revision = sessionRevision;
    const sessions = await listAgentSessions();
    if (request !== sessionRequest || revision !== sessionRevision) return;
    set((state) => ({ sessions: mergeSessions(state.sessions, sessions) }));
  },
  loadArchivedSessions: async (more = false) => {
    if (more && (get().archivedLoading || !get().archivedHasMore)) return;
    const request = ++archiveRequest;
    const revision = sessionRevision;
    const previousIds = more ? get().archivedSessionIds : [];
    const pages = more
      ? 1
      : Math.max(1, Math.ceil(get().archivedSessionIds.length / ARCHIVE_PAGE_SIZE));
    set({ archivedLoading: true, archivedError: null });
    try {
      const sessions: AgentSession[] = [];
      let hasMore = false;
      for (let page = 0; page < pages; page += 1) {
        const items = await listAgentSessions(
          undefined,
          ARCHIVE_PAGE_SIZE,
          true,
          previousIds.length + sessions.length,
        );
        sessions.push(...items);
        hasMore = items.length === ARCHIVE_PAGE_SIZE;
        if (!hasMore) break;
      }
      if (request !== archiveRequest || revision !== sessionRevision) return;
      set((state) => ({
        sessions: mergeSessions(state.sessions, sessions),
        archivedSessionIds: [
          ...new Set([...previousIds, ...sessions.map((session) => session.id)]),
        ],
        archivedLoaded: true,
        archivedHasMore: hasMore,
      }));
    } catch (error) {
      if (request === archiveRequest) {
        set({ archivedError: error instanceof Error ? error.message : String(error) });
      }
    } finally {
      if (request === archiveRequest) set({ archivedLoading: false });
    }
  },
  renameSession: async (id, title) => {
    await mutateSession(id, () => renameAgentSession(id, title.trim()));
  },
  setSessionPinned: async (id, pinned) => {
    await mutateSession(id, async () => {
      await setAgentSessionPinned(id, pinned);
      const session = get().sessions.find((item) => item.id === id);
      return session ? { ...session, pinned: pinned ? 1 : 0 } : undefined;
    });
  },
  setSessionArchived: async (id, archived) => {
    const changed = await mutateSession(id, () => setAgentSessionArchived(id, archived));
    if (!changed) return;
    if (archived && useSessionStore.getState().selectedSessionId === id) {
      useSessionStore.getState().selectSession(null);
    }
  },
}));
