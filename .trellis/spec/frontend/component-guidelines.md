# Component Guidelines

> How components are built in this project.

---

## Overview

- Function components only; **named** `export function Xxx()` (126 in `src/components` +
  `src/pages`, plus 3 `export const Xxx = memo(function Xxx(...))`).
  Default exports are used only by the three route pages in `src/pages/` (required by `lazy()`
  in `src/App.tsx`).
- Primitives come from `src/components/ui/` (shadcn-style wrappers over `@base-ui/react`);
  icons from `lucide-react`; styling is Tailwind 4 utility classes merged with `cn()`
  (`src/lib/utils.ts`).
- All user-visible text goes through `react-i18next` `useTranslation(<namespace>)`.
- Components read global state through Zustand selectors and call IO only through
  `src/lib/backend.ts` (or a `src/lib` orchestration helper).

---

## Component Structure

Typical order inside a file: imports → file-private helpers (pure functions) → exported
component(s). Hooks first inside the body, then derived values, then early returns, then JSX.

```tsx
// src/components/home/QuickPromptChips.tsx
export function QuickPromptChips() {
  const { t } = useTranslation("settings");
  const prompts = useSettingsStore((state) => state.quickPrompts);
  const setDraft = useUiStore((state) => state.setComposerDraft);

  if (prompts.length === 0) {
    return <p className="text-center text-xs text-muted-foreground">{t("general.quickPrompts")}</p>;
  }

  return (
    <div className="flex flex-wrap justify-center gap-2">
      {prompts.map((prompt) => (
        <button key={prompt.id} type="button" /* ... */ onClick={() => setDraft(prompt.prompt)}>
```

File-private pure helpers live at the top of the component file when only that file needs them
(`formatBackupTimestamp` / `buildBackupDefaultPath` in
`src/components/settings/DatabaseSection.tsx`). If they need tests, move them to `src/lib/`.

Pages are thin wrappers around a layout component:

```tsx
// src/pages/SettingsPage.tsx
import { SettingsLayout } from "@/components/settings/SettingsLayout";

export default function SettingsPage() {
  return <SettingsLayout />;
}
```

(`src/pages/ApiCallLogsPage.tsx`, ~890 lines, is the exception — a page with all logic inline.
Don't use it as a template.)

---

## Props Conventions

Two styles coexist; pick by size:

1. **Small components (≤ ~3 props)**: inline object type in the signature.

```tsx
// src/components/session/ThinkingRow.tsx
export function ThinkingRow({ items, nowMs }: { items: GroupedSessionItem[]; nowMs?: number }) {
```

2. **Larger components**: an `interface XxxProps` directly above the component (22 files do this,
   e.g. `ComposerSlashMenuProps`, `SkillImportDialogProps`). Export it only when other files need
   it:

```tsx
// src/components/settings/SettingCard.tsx
export interface SettingCardProps {
  title?: string;
  // ... description, badge, contentClassName
  icon?: ComponentType<{ className?: string }>;
  headerAction?: ReactNode;
  children: ReactNode;
  divided?: boolean;
  className?: string;
}

export function SettingCard({ title, icon: Icon, divided = false, className, ... }: SettingCardProps) {
```

Rules observed in the code:
- Destructure props in the parameter list; set defaults there (`divided = false`).
- Accept `className?: string` and merge with `cn(base, className)` for layout overrides
  (`SettingCard`, `UsageChips` in `src/components/session/UsageRow.tsx`).
- Icon props are typed `ComponentType<{ className?: string }>` and renamed to PascalCase on
  destructure (`icon: Icon`).
- Slots use `ReactNode` (`badge`, `headerAction`, `children`).
- Callbacks are named `onXxx` (`onClose` in `src/hooks/useDismissible.ts` callers,
  `onNewSession` in `useAppHotkeys`).

---

## Composition & Primitives

- Use primitives from `src/components/ui/` (`Button`, `Dialog`, `Select`, `Popover`, `Tooltip`,
  `Switch`, `DropdownMenu`, `Sheet`, `Tabs`...). They wrap `@base-ui/react` and expose variants via
  `class-variance-authority` (`buttonVariants` in `src/components/ui/button.tsx`). Don't add
  `cmdk` or Monaco (`docs/frontend.md`: "不引入 `cmdk` / Monaco").
- Settings pages compose `SettingCard` / `SettingRow` (`src/components/settings/SettingCard.tsx`)
  and `SettingFeedbackCallout` for persistent in-page errors.
- Event-stream rows wrap content in the collapsible `SegmentCard` shell
  (`src/components/session/SegmentCard.tsx`, used by `ThinkingRow.tsx`).
- Code / diff rendering always goes through `CodeBlock` / `DiffView` (Shiki via
  `src/lib/codeHighlighter.ts`), never a new highlighter.
- Native OS dialogs (`open`, `save`, `confirm`, `message`) are called directly from
  `@tauri-apps/plugin-dialog` in components (`src/components/settings/DatabaseSection.tsx`,
  `src/components/git/GitPanel.tsx`). This is the one Tauri import allowed outside `backend.ts`
  besides `isTauri`/`convertFileSrc`/`getVersion`/`plugin-updater` (see `quality-guidelines.md`).
- Guard desktop-only APIs with `isTauri()` so `npm run dev` in a browser still works
  (`src/components/session/Composer.tsx`, `src/lib/disableDefaultContextMenu.ts`).

---

## Styling Patterns

- Tailwind 4 utility classes inline; merge conditional classes with `cn()` from
  `src/lib/utils.ts` (`clsx` + `tailwind-merge`).
- Use semantic tokens (`text-muted-foreground`, `bg-card`, `border-border/70`, `bg-accent`) and the
  custom font-size tokens defined in `src/index.css` `@theme`: `text-meta`, `text-badge`,
  `text-code-sm` (they scale with `--ui-font-size`). Raw `text-[11px]` still appears in older files
  (`SettingsLayout.tsx`) — prefer the tokens in new code.
- Dark mode via `dark:` variants (`ThinkingRow.tsx`: `text-purple-500/90 dark:text-purple-400`).
- No CSS modules / styled-components; the only stylesheet is `src/index.css`.

---

## i18n in Components

```tsx
const { t } = useTranslation("sessions");                    // single namespace (most common)
const { t } = useTranslation(["settings", "common"]);        // first = default ns
t("common:saved", { defaultValue: "..." });                  // cross-namespace key
const items = t("database.backupScope.includesItems", { returnObjects: true }) as string[];
```

- Namespaces: `common nav layout sessions settings ssh git apiLogs errors`
  (`I18N_NAMESPACES` in `src/lib/i18n/index.ts`). Add a key to **both**
  `src/locales/zh-CN/<ns>.json` and `src/locales/en/<ns>.json` — `src/lib/i18n/localeKeys.test.ts`
  fails when key sets differ.
- Values that depend on `t` are memoized with `[t]` deps (`databaseFileFilters` in
  `DatabaseSection.tsx`).
- Non-React helpers that need text receive `t` as a parameter
  (`formatSessionDuration(t, seconds)` in `src/lib/sessionLines.ts`); prompt text sent to the model
  uses `promptT()` from `src/lib/promptI18n.ts`.
- **Tech debt (don't copy)**: hard-coded Chinese strings remain in some JSX
  (`CommandPalette.tsx` "选择" / "确认", `AutomationsSection.tsx` "查看运行会话",
  `ApiCallLogsPage.tsx` "{total} 条记录", `App.tsx` Suspense fallback "加载中..."), and 13 files
  pass Chinese `defaultValue`s to `t()`. New UI text must be real locale keys.

---

## Accessibility

- Every raw `<button>` in `src/components` sets an explicit `type` (`type="button"` unless it
  really submits) to avoid implicit form submit.
- Icon-only buttons get `aria-label` (88 occurrences in non-test files under `src/components`).
- Loading placeholders use `role="status"` (`src/App.tsx` Suspense fallback).
- Collapsibles expose `aria-expanded` (asserted in `ThinkingRow.test.tsx`).
- Popovers/menus close on outside pointerdown + Escape via `useDismissible`
  (`src/hooks/useDismissible.ts`).

---

## Performance

- `memo()` is used sparingly, only on hot paths of the session event stream (8 uses, all in
  `src/components/session/`: `EventStream` / `TurnBlockView` in `EventStream.tsx`,
  `AssistantMarkdown`, `BackgroundNoticeRow`, `BackgroundTaskRow`, `BackgroundProcessRow`...).
  Always the named-function form `memo(function Xxx(...))`. Don't wrap ordinary components.
- Long lists are virtualized with `@tanstack/react-virtual` + `measureElement`
  (`docs/frontend.md`, event stream).
- Heavy routes are `lazy()`-loaded in `src/App.tsx` (`SettingsPage`, `ApiCallLogsPage`).
- Select the narrowest store slice per `useXxxStore((state) => ...)` call; see
  `state-management.md`.

---

## Forbidden Patterns

| Don't | Do instead | Why / source |
|-------|------------|--------------|
| `invoke(...)` / `listen(...)` inside a component | Add a wrapper in `src/lib/backend.ts` | `docs/architecture.md`: backend.ts is the only invoke exit |
| Import `src/lib/database.ts` or any SQL plugin | Call a Tauri command via `backend.ts` | `src/lib/database.ts` throws "前端禁止直接访问 SQL" |
| Class components, `React.FC`, `forwardRef` wrappers in feature code | Plain `export function Xxx(props)` | No occurrences in `src/components` outside `ui/` |
| Default export for components | Named export | Only `src/pages/*` default-export |
| New hard-coded UI strings (Chinese or English) | `t("key")` + both locale files | `localeKeys.test.ts` parity |
| HTML5 drag-and-drop (`draggable`, `dragover`, `drop`) | Pointer events (`usePointerReorder` in `src/hooks/useWorkspaceDrag.ts`) | Tauri `dragDropEnabled` swallows `dragover`/`drop` (`docs/frontend.md`) |
| A second Dialog for plan approval / AskUserQuestion | Inline timeline cards (`PlanAskCard`, plan card) | `docs/frontend.md` session section |
| Destructuring a whole store `const { a, b } = useUiStore()` | One selector per field | No occurrences; avoids re-render on any change |

---

## Common Mistakes

- Forgetting to guard a late async result after unmount / prop change — always use the
  `cancelled` flag pattern (`hook-guidelines.md`).
- Showing one-off action errors inline: the convention is `showToast({ variant: "error",
  description: errorMessage(error) })` (`src/lib/toast.ts`), while *persistent* load failures stay
  in the page (`DatabaseSection.tsx` comment: "健康检查失败常驻页面内 ... 备份/恢复/打开目录失败走 toast").
- Keying stateful dialogs incorrectly: dialogs that must reset per entity are remounted with a
  `key` (`<MergeWorktreeDialog key={mergeDialogKey} />` in `src/App.tsx`).
