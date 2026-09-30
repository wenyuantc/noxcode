# Quality Guidelines

> Code quality standards for frontend development.

---

## Overview

Quality gates are defined in `package.json` scripts and enforced by CI
(`.github/workflows/lint.yml`, job `frontend-lint`), which runs in this order:

```bash
npm run lint           # eslint .
npm run format:check   # prettier --check .
npm run test:ci        # vitest run
npm run build          # tsc && vite build
```

`AGENTS.md` additionally requires running `npm run format:check` (and Rust clippy) after every
code change. Run all four before reporting a frontend task as done.

---

## Formatting (Prettier)

`.prettierrc`: `semi: true`, `singleQuote: false` (double quotes), `trailingComma: "all"`,
`printWidth: 100`, `tabWidth: 2` (2-space indent, no tabs), `arrowParens: "always"`,
`endOfLine: "lf"`.

`.prettierignore` excludes `*.md` and `*.json` (except `package.json` / `tsconfig*.json`), so
locale JSON files and docs are **not** auto-formatted — keep them 2-space indented by hand to
match the existing files. Use `npm run format` to fix TS/TSX formatting.

---

## Linting (ESLint flat config, `eslint.config.js`)

- Scope: `src/**/*.{ts,tsx}`, `vite.config.ts`, `vitest.config.ts`; ignores `dist`, `src-tauri`,
  `scripts`, `coverage`.
- Extends `js.configs.recommended` + `typescript-eslint` recommended + `eslint-config-prettier`.
- `react-hooks/rules-of-hooks`: error; `react-hooks/exhaustive-deps`: warn. React Compiler-style
  hook rules are intentionally not enabled (comment in `eslint.config.js`).
- `@typescript-eslint/no-unused-vars`: error, except names prefixed with `_`
  (args, vars, caught errors). Use `_query` style for intentionally unused params
  (`src/lib/database.ts`).
- `no-empty` allows empty `catch {}` — used for best-effort browser fallbacks
  (`syncWindowTitle` in `src/lib/i18n/index.ts`).
- `no-explicit-any` and `no-empty-object-type` are off (see `type-safety.md` for why we still
  avoid `any`).

---

## Forbidden Patterns

| Pattern | Why | Source |
|---------|-----|--------|
| Direct SQLite access from the frontend (adding `@tauri-apps/plugin-sql` to `package.json` — it is intentionally absent — or using `getDb/select/execute` from `src/lib/database.ts`) | Data-flow iron rule; the stub throws and capabilities grant no `sql:*` permission | `AGENTS.md`, `src/lib/database.ts`, `src-tauri/capabilities/default.json` |
| `invoke` / `listen` imported outside `src/lib/backend.ts` | backend.ts is the only IPC exit | `docs/architecture.md` |
| Tauri imports outside backend.ts other than: `isTauri` / `convertFileSrc` (`@tauri-apps/api/core`), `@tauri-apps/plugin-dialog`, `@tauri-apps/api/app` `getVersion` and `@tauri-apps/plugin-updater` (both only in `src/lib/appUpdate.ts`), dynamic `import()` of `@tauri-apps/api/window` / `webview` / `plugin-opener` | Keep native surface small and browser dev working | current imports in `src/` |
| HTML5 drag-and-drop | Tauri native drag-drop swallows `dragover` / `drop` | `docs/frontend.md` |
| Adding `cmdk`, Monaco, another highlighter | UI base is `@base-ui/react` + Shiki | `docs/frontend.md` |
| Hard-coded user-visible strings | Locale parity test + en/zh UI | `src/lib/i18n/localeKeys.test.ts` |
| Writing business data to `localStorage` | Stores only cache Rust state; prefs only | `docs/frontend.md` |

---

## Testing Requirements

- Framework: Vitest (`vitest.config.ts`), `environment: "node"`, includes
  `src/**/*.test.{ts,tsx}` and `scripts/**/*.test.ts`. 92 test files exist under `src/`.
- **Colocate** tests next to the source: `src/lib/gitHelpers.ts` ↔ `src/lib/gitHelpers.test.ts`,
  `src/stores/gitStore.ts` ↔ `gitStore.test.ts`, `src/components/git/GitPanel.tsx` ↔
  `GitPanel.test.tsx`. No `__tests__/` directories.
- New/changed pure logic in `src/lib/` should come with a test. Most `src/lib/*.ts` files have
  one; the untested ones are declarations/wrappers (`backend.ts`, `types.ts`, `database.ts`,
  `codeThemes.ts`, `shortcuts.ts`, `codeHighlighter.ts`) plus some existing gaps
  (`nativeLifecycle.ts`, `nativePlanQuestion.ts`, `openLocalWorkspace.ts`, `promptI18n.ts`,
  `theme.ts`) — don't treat the gaps as precedent.
- Store changes: test via `useXxxStore.getState()` actions with `@/lib/backend` mocked.
- Component tests render to a string — there is **no** Testing Library / jsdom:

```tsx
// src/components/session/ThinkingRow.test.tsx
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import i18n from "@/lib/i18n";

const html = renderToStaticMarkup(
  <I18nextProvider i18n={i18n}>
    <ThinkingRow items={[/* ... */]} nowMs={Date.parse("2026-01-01T00:00:00Z")} />
  </I18nextProvider>,
);
expect(html).toContain('aria-expanded="false"');
```

  Either render with the real `i18n` (asserts real zh-CN text) or mock `react-i18next` so `t`
  returns the key (`src/components/settings/DatabaseSection.test.tsx`).
- Mock at module boundaries with `vi.mock("@/lib/backend", ...)` (25 files),
  `vi.mock("@/stores/...")`, `vi.mock("@tauri-apps/plugin-dialog")`; use `vi.hoisted` for shared
  mock state and `vi.mocked(fn)` for typing.
- Reset store state in `beforeEach` with `useXxxStore.setState({...})`
  (`src/stores/gitStore.test.ts`).
- Test async ordering with a hand-made `deferred()` promise (`gitStore.test.ts`) rather than
  timers.
- For orchestration code, prefer dependency injection over module mocks:
  `submitSessionPrompt(input, api = defaultApi)` in `src/lib/sessionSubmission.ts` is tested by
  passing a fake `api()`.
- Locale changes: `src/lib/i18n/localeKeys.test.ts` must stay green (same key set in `zh-CN` and
  `en` for every namespace).

---

## Documentation Sync

`docs/frontend.md` describes routes, directory map, store table, persisted keys, events and
shortcuts in detail. When a change alters any of those, update `docs/frontend.md` (and
`docs/architecture.md` for new IPC commands/events) in the same task. Docs are written in
Chinese; code comments are mixed Chinese/English (e.g. Chinese in `src/lib/toast.ts`,
`DatabaseSection.tsx`; English in `src/stores/gitStore.ts`, `src/lib/workspaceOrder.ts`) — either
is accepted, match the surrounding file.

---

## Code Review Checklist

- [ ] No `invoke`/`listen` outside `src/lib/backend.ts`; new commands have a typed wrapper and DTOs
      in `src/lib/types.ts` whose casing matches the Rust `serde` attributes.
- [ ] No SQL / `database.ts` usage in the frontend.
- [ ] Components: named export, PascalCase file, props typed inline or via `XxxProps`, `cn()` for
      class merging, `ui/` primitives reused.
- [ ] All new text is in both `src/locales/zh-CN/<ns>.json` and `src/locales/en/<ns>.json`.
- [ ] Store reads use per-field selectors; handlers/effects use `getState()`.
- [ ] Async effects / listeners use the `cancelled` guard; stores guard stale responses.
- [ ] New persisted keys use the `noxcode:` prefix and are listed in `docs/frontend.md`.
- [ ] Raw `<button>` has `type`; icon-only controls have `aria-label`.
- [ ] Desktop-only APIs guarded with `isTauri()`.
- [ ] Colocated `*.test.ts(x)` added/updated; `npm run lint`, `npm run format:check`,
      `npm run test:ci`, `npm run build` pass.

---

## Common Mistakes

- Running only `vitest` (watch mode, `npm run test`) and never `npm run test:ci` / `npm run build`
  — `tsc` catches `noUnusedLocals` / `noUnusedParameters` that Vitest doesn't.
- Formatting a locale JSON with a different indent — Prettier won't fix it (ignored), and diffs get
  noisy.
- Adding a DOM-dependent test (`document.querySelector`, events) — the environment is `node`;
  extract the logic or render to static markup.
