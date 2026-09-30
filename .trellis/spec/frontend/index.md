# Frontend Development Guidelines

> Conventions for the Vite + React 19 + TypeScript + Zustand frontend (`src/`) of the noxcode
> Tauri 2 desktop app. Every rule here is extracted from `AGENTS.md`, `docs/frontend.md`,
> `docs/architecture.md`, the lint/format/test configs, or real code under `src/`.

---

## Overview

- Data flow iron rule: `React (UI) → src/lib/backend.ts → Tauri IPC command → Rust → SQLite`.
  The frontend never touches SQLite (`src/lib/database.ts` is a hard-fail stub).
- Zustand stores only cache state fetched from Rust (plus UI prefs in `localStorage`).
- UI primitives: `@base-ui/react` + shadcn-style tokens in `src/components/ui/`, Tailwind CSS 4.
- i18n: `i18next` / `react-i18next`, locales `zh-CN` (fallback) and `en`, 9 namespaces.
- Tests: Vitest, `environment: "node"`, colocated `*.test.ts(x)`; components are rendered with
  `renderToStaticMarkup`, no Testing Library.

---

## Guidelines Index

| Guide | Description | Status |
|-------|-------------|--------|
| [Directory Structure](./directory-structure.md) | Module organization and file layout | Filled |
| [Component Guidelines](./component-guidelines.md) | Component patterns, props, composition | Filled |
| [Hook Guidelines](./hook-guidelines.md) | Custom hooks, data fetching patterns | Filled |
| [State Management](./state-management.md) | Local state, global state, server state | Filled |
| [Quality Guidelines](./quality-guidelines.md) | Code standards, forbidden patterns | Filled |
| [Type Safety](./type-safety.md) | Type patterns, validation | Filled |

---

## Pre-Development Checklist

Read these before writing code, by kind of change:

| Change | Read |
|--------|------|
| Any frontend change | `quality-guidelines.md` (commands, forbidden patterns), `directory-structure.md` |
| New/changed Tauri command call or event | `type-safety.md` (IPC types), `hook-guidelines.md` (event subscription), backend `command-guidelines.md` |
| New component / dialog / settings section | `component-guidelines.md`, `directory-structure.md` |
| New or changed store / cached backend data / `localStorage` key | `state-management.md` |
| New custom hook or effect that fetches data | `hook-guidelines.md` |
| New user-visible text | `component-guidelines.md` (i18n section) — add keys to **both** `src/locales/zh-CN` and `src/locales/en` |
| New pure helper in `src/lib` | `directory-structure.md`, `quality-guidelines.md` (colocated test) |

Also check `docs/frontend.md` for feature-level behavior (routes, store responsibilities,
session wiring) before changing an existing feature — keep it in sync when behavior changes.

Before finishing, run: `npm run lint`, `npx tsc --noEmit` (or `npm run build`),
`npm run test:ci`, `npm run format:check`.

---

**Language**: All documentation should be written in **English**.
