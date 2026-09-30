# Directory Structure

> How frontend code is organized in this project.

---

## Overview

Sources: `AGENTS.md` (project structure + naming), `docs/frontend.md` (directory map), real tree
under `src/`. Organization is **by layer first** (`pages/`, `components/`, `hooks/`, `stores/`,
`lib/`), then **by feature** inside `components/`. There is no `features/` folder and no barrel
`index.ts` files (the only `index.ts` is the i18n module `src/lib/i18n/index.ts`) — import each
module by its full path.

The `@/*` alias maps to `./src/*` (`tsconfig.json` `paths`, `vitest.config.ts` `resolve.alias`).

---

## Directory Layout

```
src/
├── main.tsx                 # i18n init → applyTheme / applyCodeAppearance → disable WebView menu → <App/>
├── App.tsx                  # BrowserRouter, 4 routes, global dialogs (permission / merge / SSH trust)
├── index.css                # Tailwind 4 @theme tokens (text-meta, text-badge, text-code-sm, ...)
├── pages/                   # Route-level entry points, default export, very thin
│   ├── WorkspacePage.tsx    # "/" → <AppShell/>
│   ├── SettingsPage.tsx     # "/settings/:section" → <SettingsLayout/> (lazy)
│   └── ApiCallLogsPage.tsx  # "/api-logs" (lazy)
├── components/
│   ├── ui/                  # shadcn-style primitives on @base-ui/react (button, dialog, select, toast...)
│   ├── layout/              # AppShell, SidebarTree, SidebarCommands, SidebarFooter
│   ├── session/             # Composer, EventStream, *Row, *Picker, *Dialog for the chat view
│   ├── settings/            # SettingsLayout + one *Section / *Tab / *Card per settings page
│   ├── git/                 # GitSidebar, GitPanel, DiffView, CheckpointTimeline, dialogs
│   ├── code/                # CodeBlock, CodePreview (Shiki)
│   ├── apiLogs/ command/ home/ ssh/ workspace/   # small feature folders
│   └── profile/             # EMPTY — leftover, do not add files here
├── hooks/                   # useNativeEvents, useNativeSteer, useSshTrustEvents, useAppHotkeys,
│                            # useDismissible, useWorkspaceDrag
├── stores/                  # one Zustand store per file: uiStore, workspaceStore, sessionStore,
│                            # channelStore, settingsStore, updateStore, gitStore, steerStore
├── lib/
│   ├── backend.ts           # THE ONLY IPC exit (invoke/listen wrappers)
│   ├── types.ts             # all IPC DTO types mirroring Rust serde structs
│   ├── database.ts          # hard-fail stub, never import
│   ├── i18n/                # index.ts (init + namespaces), locale.ts, localeKeys.test.ts
│   └── *.ts                 # pure helpers / orchestration (sessionLines, workspaceOrder, toast...)
└── locales/{zh-CN,en}/      # common, nav, layout, sessions, settings, ssh, git, apiLogs, errors .json
```

---

## Module Organization

### Where new code goes

| You are adding | Put it in | Real example |
|----------------|-----------|--------------|
| A new route | `src/pages/XxxPage.tsx` (default export) + `<Route>` in `src/App.tsx` | `src/pages/ApiCallLogsPage.tsx`, lazily imported in `src/App.tsx` |
| A settings page section | `src/components/settings/XxxSection.tsx`, register in `SECTION_META` + `GROUPS` of `src/components/settings/SettingsLayout.tsx` | `src/components/settings/DatabaseSection.tsx` |
| A chat/event-stream element | `src/components/session/XxxRow.tsx` | `src/components/session/ThinkingRow.tsx`, `UsageRow.tsx` |
| A reusable primitive | `src/components/ui/<kebab-name>.tsx` | `src/components/ui/button.tsx` |
| A Tauri command call / event listener | a function in `src/lib/backend.ts` + DTOs in `src/lib/types.ts` | `getGitStatus`, `onSshHostTrustRequest` in `src/lib/backend.ts` |
| Logic without React (parsing, ordering, formatting) | `src/lib/camelCase.ts` + `src/lib/camelCase.test.ts` | `src/lib/workspaceOrder.ts` (pure, used by `src/hooks/useWorkspaceDrag.ts`) |
| Cross-store orchestration called from events/components | `src/lib/camelCase.ts` | `src/lib/nativeLifecycle.ts`, `src/lib/sessionSubmission.ts` |
| App-wide cached state | `src/stores/xxxStore.ts` | `src/stores/gitStore.ts` |

Feature-local subcomponents stay **in the same file** as long as they are only used there
(e.g. `FilePreviewPanel` inside `src/components/git/GitDiffDialog.tsx`). They may be exported
only so the colocated test can render them (`SettingsBrandFooter` in
`src/components/settings/SettingsLayout.tsx` is imported by `SettingsLayout.test.tsx`).

### Import style

Order (not enforced by a tool, but consistent across files): external packages → blank line →
`@/...` absolute imports → sibling `./X` imports last. Cross-folder imports use `@/`; same-folder
imports use `./` (no `../` imports exist under `src/components`).

```tsx
// src/components/layout/AppShell.tsx
import { useEffect, useRef } from "react";

import { CommandPalette } from "@/components/command/CommandPalette";
import { useAppHotkeys } from "@/hooks/useAppHotkeys";
import { useUiStore } from "@/stores/uiStore";
import { SidebarCommands } from "./SidebarCommands";
```

---

## Naming Conventions

From `AGENTS.md`: "React components, pages and dialog files use PascalCase; stores, utilities and
module helper files use camelCase."

| Kind | Convention | Examples |
|------|-----------|----------|
| Components / pages / dialogs | `PascalCase.tsx` | `SessionHeader.tsx`, `GitDiffDialog.tsx`, `SettingsPage.tsx` |
| Pages | `XxxPage.tsx` | `WorkspacePage.tsx`, `ApiCallLogsPage.tsx` |
| Dialogs | `XxxDialog.tsx` | `MergeWorktreeDialog.tsx`, `SkillImportDialog.tsx` |
| Stores | `xxxStore.ts`, hook `useXxxStore` | `sessionStore.ts` → `useSessionStore` |
| Hooks | `useXxx.ts` | `useNativeEvents.ts`, `useDismissible.ts` |
| Lib helpers | `camelCase.ts` | `gitHelpers.ts`, `composerSlash.ts` |
| Locale files | `<namespace>.json`, same set in both locales | `src/locales/en/sessions.json` |
| Tests | `<source>.test.ts(x)` next to the source | `src/stores/gitStore.test.ts` |

**Known exception**: shadcn primitives in `src/components/ui/` are lowercase/kebab-case
(`button.tsx`, `dropdown-menu.tsx`, `scroll-area.tsx`). Keep that style *only* inside `ui/`.

**Known inconsistency**: a file may export a differently-named component, e.g.
`src/components/session/UsageRow.tsx` exports `UsageChips`. Don't copy this; name new files after
their main export.

---

## Common Mistakes

- Adding an `index.ts` barrel — none exist; import `@/components/session/Composer` directly.
- Calling `invoke` from a component or store — add a wrapper to `src/lib/backend.ts` instead.
- Putting React-free logic inside a component file where it can't be unit-tested in the node
  test environment — move it to `src/lib/`.
- Placing a new file in the empty `src/components/profile/` folder.
