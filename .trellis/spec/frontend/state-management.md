# State Management

> How state is managed in this project.

---

## Overview

- **Global state**: Zustand 5, one `create<XxxState>()` store per file in `src/stores/`.
  No middleware (`persist`, `immer`, `devtools`, `subscribeWithSelector` are not used anywhere).
- **Server state**: there is no React Query. The "server" is Rust behind Tauri IPC; stores cache
  what `src/lib/backend.ts` returns. `docs/frontend.md`: "Zustand 只缓存从 Rust 取回的状态".
- **Persistence**: UI preferences only, written to `localStorage` by hand inside store actions,
  keys prefixed `noxcode:`. Business data is never persisted in the frontend and never written to
  SQLite from the frontend.
- **Local state**: `useState` / `useRef` in components for form fields, loading flags, dialogs.
- **Context**: only `ToastStackContext` in `src/components/ui/toast.tsx`. Don't introduce React
  context for app state — use a store.

---

## State Categories

| Store (`src/stores/`) | Holds | Persisted keys |
|-------|-------|----------------|
| `uiStore` | sidebar, command palette, Git drawer, Composer draft / plan mode / thinking level, theme, code appearance | `noxcode:sidebar-width`, `noxcode:sidebar-collapsed`, `noxcode:git-panel-width`, `noxcode:composer-plan-mode`, `noxcode:composer-thinking-level`, theme + code-appearance keys |
| `workspaceStore` | workspaces, session list, archived list, expansion, ordering, health | `noxcode:active-workspace`, `noxcode:workspace-expanded`, `noxcode:workspace-order`, `noxcode:session-order` |
| `channelStore` | AI channels, model catalog, default channel/model | `noxcode:active-model` |
| `sessionStore` | live sessions, event lines, turn state, usage, permission / plan requests (all keyed by session id) | — |
| `settingsStore` | native / network / AI settings, quick prompts | — |
| `updateStore` | desktop update check / download / restart state | — |
| `gitStore` | per-workspace pull state, `revision` counter to trigger Git UI refresh | — |
| `steerStore` | steer snapshots / lifecycle per session, version-filtered | — |

The table in `docs/frontend.md` ("Store" section) is the feature-level source of truth; update it
when you add a store or a persisted key.

---

## Store Shape

State fields and actions live in one interface; actions are `async` when they call the backend.

```ts
// src/stores/gitStore.ts
export type GitPullState =
  | { status: "pulling" }
  | { status: "success"; result: GitPullResult }
  | { status: "error"; error: string };

interface GitState {
  pulls: Record<string, GitPullState | undefined>;
  revision: number;
  bumpRevision: () => void;
  pull: (workspaceId: string) => Promise<void>;
}

// Pulls outlive the sidebar so reopening it cannot start a duplicate operation.
export const useGitStore = create<GitState>((set, get) => ({
```

Conventions:
- Per-entity state is a `Record<string, T>` keyed by id (`pulls[workspaceId]`,
  `sessionStore.lines[sessionId]`, `liveBySession`, `turnState`, `usage`...). Not `Map`.
- Async status is a discriminated union (`GitPullState`) or explicit flags
  (`archivedLoading`, `archivedError` in `workspaceStore`).
- Loading actions are named `load()` / `refreshXxx()`; they fetch in parallel with `Promise.all`
  and `set` once:

```ts
// src/stores/settingsStore.ts
load: async () => {
  const [native, network, ai, quickPrompts] = await Promise.all([
    getNativeSettings(), getNetworkSettings(), getAiSettings(), getQuickPrompts(),
  ]);
  set({ native, network, ai: normalizeAiSettings(ai), quickPrompts });
},
```

- Setters after a successful mutation are `setXxx(value)` (`setNative`, `setAi`) — the settings
  component calls the backend `updateXxx`, then pushes the returned value into the store
  (`updateNativeSettings({ lsp_enabled: checked }).then((updated) => setNative(updated))` in
  `src/components/settings/LspSection.tsx`).
- Event handlers called from `useNativeEvents` are `onXxx(payload)` and may return `boolean`
  ("accepted / changed") so the caller can decide on follow-up work
  (`onStarted`, `onTurnState`, `onExit`, `onConfiguration` in `sessionStore`).
- Pure normalization runs before `set` (`normalizeAiSettings` from `src/lib/aiSettings.ts`,
  `resolveSelection` in `channelStore`).

---

## Reading State in Components

One selector per value; never destructure the whole store.

```tsx
// src/components/layout/AppShell.tsx
const collapsed = useUiStore((state) => state.sidebarCollapsed);
const setWidth = useUiStore((state) => state.setSidebarWidth);
const selected = useSessionStore((state) => state.selectedSessionId);
```

- Derived selectors may compute from state (`useSessionStore((state) =>
  mergeWorktreeDialogKey(state.worktreeMergePrompt))` in `src/App.tsx`). Return primitives or
  stable references — `useShallow` is not used, so a selector returning a new object/array each
  call causes re-renders.
- Inside event handlers, effects and non-React code, read/write with `useXxxStore.getState()`
  (~90 non-test uses in `src/components`), e.g. `useSessionStore.getState().selectSession(null)`.

---

## Persistence Pattern (localStorage)

Hand-written, SSR-guarded read helpers at module top; writes happen inside the action that
changes the value.

```ts
// src/stores/channelStore.ts
const ACTIVE_KEY = "noxcode:active-model";

function readStored(): StoredSelection | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(ACTIVE_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as StoredSelection;
    if (parsed.channelId && parsed.modelId) return parsed;
  } catch {
    return null;
  }
  return null;
}
```

- Clamp / validate on read *and* write (`setSidebarWidth` clamps to 200–480 in
  `src/stores/uiStore.ts`; `readExpanded` in `workspaceStore.ts` drops non-boolean values).
- Booleans are stored as `"1"` / `"0"` (`noxcode:sidebar-collapsed`, `noxcode:composer-plan-mode`).
- `localStorage` is only touched in `src/stores/*` and `src/lib/{theme,codeAppearance}.ts`,
  `src/lib/i18n/locale.ts` (`noxcode:locale`) — never in components or hooks.

---

## Async Races & Stale Results

Multiple IPC calls can be in flight; late responses must not overwrite newer state. Existing
techniques — reuse them:

1. **Request counter + revision** (module-level `let`):

```ts
// src/stores/workspaceStore.ts
refreshSessions: async () => {
  const request = ++sessionRequest;
  const revision = sessionRevision;
  const sessions = await listAgentSessions();
  if (request !== sessionRequest || revision !== sessionRevision) return;
  set((state) => ({ sessions: mergeSessions(state.sessions, sessions) }));
},
```

2. **Merge by id instead of replace** (`mergeSessions` from `src/lib/sessionActions.ts`), so an
   older list can't erase an optimistic update.
3. **Dedupe in-flight work** in the store, not the component
   (`if (get().pulls[workspaceId]?.status === "pulling") return;` in `gitStore.ts`).
4. **Monotonic revisions from the backend** (`steerStore.ts` rejects snapshots / events with a
   lower `revision`; `sessionStore` tracks `configurationRevisionBySession`).

---

## Cross-Store Access

Stores call each other through `getState()` at action time:
`sessionStore.ts` reads `useWorkspaceStore` / `useChannelStore` in `hydrateUsage`, and
`workspaceStore.ts` calls `useSessionStore.getState().selectSession(null)` when archiving the
selected session. This is a **circular import** between `sessionStore` and `workspaceStore`; it
works because access happens inside functions, never at module top level. Keep it that way.

Logic spanning several stores + backend calls goes in `src/lib/` (e.g. `src/lib/nativeLifecycle.ts`,
`src/lib/sessionConfiguration.ts`, `src/lib/worktreeMergePrompt.ts`), not in a component.

---

## Common Mistakes

- Treating a store as source of truth and "saving" it — the backend is authoritative; after a
  mutation, use the value the command returns or call `load()` / `refreshXxx()`.
- Adding a persisted key without documenting it in `docs/frontend.md` or without the
  `noxcode:` prefix (legacy exceptions: `theme`, `theme-mode`, which must match the anti-flash
  script in `index.html`).
- Deciding session behavior from cached UI state: `docs/frontend.md` notes the frontend must not
  use `liveBySession` staleness to decide whether to start a new session — `submitSessionPrompt`
  always resumes the selected `session_record_id` and Rust decides.
- Accessing another store at module top level (breaks with the circular import above).
