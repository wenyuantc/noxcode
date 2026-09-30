# Hook Guidelines

> How hooks are used in this project.

---

## Overview

- Shared hooks live in `src/hooks/` as `useXxx.ts`, one exported hook per file (plus private
  helpers): `useNativeEvents`, `useNativeSteer`, `useSshTrustEvents`, `useAppHotkeys`,
  `useDismissible`, `useWorkspaceDrag` (which also exports `usePointerReorder`).
- A hook used by only one component may stay private in that component file
  (`usePlanApproval` in `src/components/session/PlanRow.tsx`, `useToastStackContext` in
  `src/components/ui/toast.tsx`).
- **No data-fetching library** (no React Query / SWR). Backend data is fetched either by a store
  action (`load()`, `refreshSessions()`) or by a `useEffect` with a `cancelled` flag.
- Lint: `react-hooks/rules-of-hooks` is `error`, `react-hooks/exhaustive-deps` is `warn`
  (`eslint.config.js`); React-Compiler-style rules are intentionally off.

---

## Custom Hook Patterns

### 1. Tauri event subscription (the canonical pattern)

`backend.ts` `onXxx(callback)` returns `Promise<UnlistenFn>`. Because the promise may resolve after
unmount, every subscriber uses the `cancelled` + `unlistens` pattern:

```ts
// src/hooks/useSshTrustEvents.ts
useEffect(() => {
  let cancelled = false;
  const unlistens: Array<() => void> = [];
  void onSshHostTrustRequest((value) => setPrompt(value)).then((fn) => {
    if (cancelled) fn();
    else unlistens.push(fn);
  });
  return () => {
    cancelled = true;
    unlistens.forEach((fn) => fn());
  };
}, []);
```

`src/hooks/useNativeEvents.ts` factors this into a local `track(promise)` helper for 17 events
and is mounted **exactly once** in `AppEffects` (`src/App.tsx`). Its handlers write into stores via
`useXxxStore.getState()` (not via selectors), so the effect has `[]` deps. New `native-*` events
must be added there, not subscribed ad hoc in components.

### 2. One-shot fetch in an effect

```ts
// src/components/git/GitDiffDialog.tsx (FilePreviewPanel)
useEffect(() => {
  let cancelled = false;
  setPreview(null);
  setError(null);
  // ... const request: Promise<GitFilePreview> = scope === "auto" ? getGitFilePreview(...) : getGitFileDiff(...)
  void request.then(
    (next) => { if (!cancelled) setPreview(next); },
    (reason) => { if (!cancelled) setError(reason instanceof Error ? reason.message : String(reason)); },
  );
  return () => { cancelled = true; };
}, [workspaceId, path, scope, oldPath, sessionId, attempt]);
```

- Reset state at the start of the effect so stale data never shows for the new key.
- A retry counter (`attempt`) in deps is how "retry" buttons re-run the effect.
- Same pattern: `getAppVersion()` in `src/components/settings/SettingsLayout.tsx`,
  `getNativeSteerSnapshot` in `src/hooks/useNativeSteer.ts`.

### 3. Kick off store loads on mount

```ts
// src/components/layout/AppShell.tsx
useEffect(() => {
  void Promise.all([
    useWorkspaceStore.getState().load(),
    useChannelStore.getState().load(),
    useSettingsStore.getState().load(),
  ]);
}, []);
```

Fire-and-forget promises are prefixed with `void` (used consistently across `src/`).

### 4. DOM listener hooks

`src/hooks/useDismissible.ts` takes `(open, onClose, containerRef)`, attaches `pointerdown` +
`keydown` on `document` only while `open`, and removes them in cleanup. Callers must pass a stable
or intentionally changing `onClose` (it is in deps).

### 5. Hooks that wrap a library with app config

`src/hooks/useAppHotkeys.ts` reads bindings from `GLOBAL_SHORTCUTS` / `shortcutKeys()` in
`src/lib/shortcuts.ts` and calls `react-hotkeys-hook`'s `useHotkeys`. Add new global shortcuts to
`src/lib/shortcuts.ts` first; component-scoped keys (Composer `Shift+Tab`) are not in that table.

---

## Return Shape

- Hooks with several outputs return a **plain object**, not a tuple:
  `useSshTrustEvents()` → `{ prompt, setPrompt, changed, setChanged }`;
  `useNativeSteer(sessionId)` → `{ snapshot, busy, error, submit, canSubmit }`.
- Effect-only hooks return nothing (`useNativeEvents`, `useAppHotkeys`, `useDismissible`).
- Hooks accept primitives / callbacks as positional args when few
  (`useAppHotkeys(onNewSession, onOpenWorkspace)`), an `options` object when many
  (`usePointerReorder(options)`, `useWorkspaceDrag(options)`).

---

## Keeping Hooks Thin

Put the logic in `src/lib/` and keep the hook as React glue — this is what makes it testable in
the node test environment:

- `src/hooks/useWorkspaceDrag.ts` handles pointer capture; drop-position math lives in the pure
  `src/lib/workspaceOrder.ts` (tested by `workspaceOrder.test.ts`).
- `src/hooks/useNativeSteer.ts` delegates to `prepareSteerAttempt` / `submitSteerAttempt` in
  `src/lib/nativeSteer.ts`.
- `useNativeEvents` delegates turn-state / exit handling to `handleNativeTurnState` /
  `handleNativeExit` in `src/lib/nativeLifecycle.ts`.

Guard against double submission with a ref, not only state (`pending.current` in
`useNativeSteer.ts`), because state updates are async.

---

## Naming Conventions

- `useXxx` for every hook; file name equals the main hook name (`useDismissible.ts`).
  Exception: `useWorkspaceDrag.ts` also exports `usePointerReorder`.
- Event subscription hooks are named after the domain, `use<Domain>Events`
  (`useNativeEvents`, `useSshTrustEvents`).
- Store hooks are `useXxxStore` and live in `src/stores/`, not `src/hooks/`.

---

## Testing Hooks

There is no `@testing-library/react` and no DOM environment (`vitest.config.ts`:
`environment: "node"`). Hooks are tested by mocking React:

```ts
// src/hooks/useNativeEvents.test.ts
vi.mock("react", async (original) => ({
  ...(await original<typeof import("react")>()),
  useEffect: (effect: () => void) => effect(),
}));
vi.mock("@/lib/backend", () => Object.fromEntries([...names].map((name) => [
  name, (callback) => { callbacks.set(name, callback); return Promise.resolve(() => undefined); },
])));
```

`src/hooks/useNativeSteer.test.ts` goes further and mocks `useState` / `useRef` with a slot
harness. Prefer moving logic to `src/lib` over growing such harnesses.

---

## Common Mistakes

- Calling `unlisten` only in cleanup without the `cancelled` check — leaks a listener when the
  `listen` promise resolves after unmount. Existing instance of this debt:
  `onNativeMcpOAuth` in `src/components/settings/McpSettingsTab.tsx` (`return () => unlisten?.()`).
  Don't copy it; use the `cancelled` pattern above.
- Subscribing to a session-wide `native-*` event in a component instead of `useNativeEvents` —
  duplicates handlers and races the store. Only screen-scoped events whose result is local UI
  (MCP OAuth callback in `McpSettingsTab.tsx`) are subscribed inside a component.
- Reading store state through a selector inside a subscription callback — use
  `useXxxStore.getState()` in callbacks and effects, selectors only for rendering.
- Silencing `exhaustive-deps`: only two `eslint-disable-next-line react-hooks/exhaustive-deps`
  exist (`src/components/settings/McpSettingsTab.tsx`, `src/pages/ApiCallLogsPage.tsx`). Don't add
  more without a comment explaining why.
