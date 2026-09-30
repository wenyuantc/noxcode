# Type Safety

> Type safety patterns in this project.

---

## Overview

- TypeScript 5.8, `strict: true`, `noUnusedLocals`, `noUnusedParameters`,
  `noFallthroughCasesInSwitch`, `isolatedModules`, `moduleResolution: "bundler"`, `noEmit`
  (`tsconfig.json`). `npm run build` runs `tsc` first, so type errors fail the build.
- No runtime schema library (no zod / valibot). Backend payloads are trusted to match
  `src/lib/types.ts`; untrusted input (localStorage, user-pasted JSON) is validated by hand.
- ESLint turns `@typescript-eslint/no-explicit-any` **off**, but non-test source has zero
  `: any` / `as any`. Keep it that way — use `unknown` + narrowing.
- No `enum`s. Closed sets are string-literal unions.

---

## Type Organization

| Where | What | Example |
|-------|------|---------|
| `src/lib/types.ts` (~1700 lines, ~150 interfaces) | Every IPC DTO: command inputs, results, event payloads | `Workspace`, `CreateWorkspaceInput`, `GitFilePreview`, `NativeTurnState` |
| Next to the module that owns it | Types used only by one lib/store/component | `GitPullState` in `src/stores/gitStore.ts`, `SessionSubmissionInput` in `src/lib/sessionSubmission.ts`, `SettingCardProps` in `SettingCard.tsx` |
| `src/lib/i18n/locale.ts` | `AppLocale = "zh-CN" \| "en"` | — |

`src/lib/types.ts` groups types by domain (SSH, Git, worktree, channels, native session...) with
no sub-files. Add new IPC types there, next to their domain siblings.

Always use type-only imports for types:

```ts
import type { GitPullResult } from "@/lib/types";
import { hydrateSessionLine, parseSubagentTag, type RawSessionLine } from "@/lib/sessionLines";
```

---

## IPC Types Mirror Rust

DTO field names match the Rust `serde` output **exactly**. Rust structs keep their default
(snake_case) field names with no `rename_all`, so TS fields are snake_case. Enums use
`#[serde(rename_all = "snake_case")]` (all 31 occurrences in `src-tauri/src` are on enums), so
variant tags are snake_case string literals too:

```ts
// src/lib/types.ts
export interface Workspace {
  id: string;
  name: string;
  workspace_type: WorkspaceType;
  repo_path: string | null;
  ssh_config_id: string | null;
  remote_repo_path: string | null;
  created_at: string;
  updated_at: string;
}
```

Exceptions: the MCP OAuth structs in `src-tauri/src/native/mcp_oauth.rs` are
`rename_all = "camelCase"`, so `McpOAuthEvent` has `serverId`. Always check the Rust struct
before writing the TS interface — never guess the casing.

Conventions inside `types.ts`:
- Rust `Option<T>` → `T | null` by default. Optional `?:` appears on (a) input/update types where
  omitted fields mean "unchanged" (`UpdateSshConfigInput`), and (b) response fields that Rust may
  omit (`skip_serializing_if`, 37 uses in `src-tauri/src`) or that were added by later migrations
  (`AgentSession.title?`, `pending_plan_json?`, `approved_plan_json?`). Consumers must handle
  `undefined` and `null` both — use `?? null` / optional chaining.
- Timestamps are `string` (parsed on display with `parseDateValue` / `formatDate` in
  `src/lib/utils.ts`).
- Command input types are `CreateXxxInput` / `UpdateXxxInput` / `XxxInput`
  (`CreateSshConfigInput`, `UpdateAiChannelInput`, `ListNativeApiCallLogsInput`).
- Closed sets are exported unions (`GitNumstatScope = "worktree" | "staged" | "upstream"`), with a
  constant array + guard when the UI iterates or validates them:

```ts
// src/lib/types.ts
export type NativePermissionMode = "default" | "edit" | "build" | "yolo";

export const NATIVE_PERMISSION_MODES: NativePermissionMode[] = ["default", "edit", "build", "yolo"];

export function isNativePermissionMode(value: unknown): value is NativePermissionMode {
  return value === "default" || value === "edit" || value === "build" || value === "yolo";
}
```

---

## Typing `backend.ts` Wrappers

Every wrapper declares its return type on the function; `invoke` itself is called without a
generic (the declared `Promise<T>` provides the type). Arguments are camelCase — Tauri maps them to
the Rust command's snake_case parameters. Optional ids are normalized to `null`:

```ts
// src/lib/backend.ts
export function getGitStatus(
  workspaceId: string,
  untrackedMode?: string,
  sessionId?: string | null,
): Promise<GitStatus> {
  return invoke("get_git_status", { workspaceId, untrackedMode, sessionId: sessionId ?? null });
}

export function createSshConfig(payload: CreateSshConfigInput): Promise<SshConfig> {
  return invoke("create_ssh_config", { payload });
}
```

Event wrappers type the payload with `listen<T>` and return `Promise<UnlistenFn>`:

```ts
export function onSshHostTrustRequest(
  callback: (prompt: SshHostTrustPrompt) => void,
): Promise<UnlistenFn> {
  return listen<SshHostTrustPrompt>("ssh-host-trust-request", (event) => {
    callback(event.payload);
  });
}
```

Commands whose output is localized pass `locale: getLocalePreference()`
(`resolveSessionWorktreeMerge`, `getQuickPrompts`, `generateGitCommitMessage` in
`src/lib/backend.ts`).

---

## Discriminated Unions

Variant data uses a literal discriminant (`kind`, `status`) and is narrowed with `===` / `switch`:

```ts
// src/lib/types.ts
export type GitFilePreview =
  | { kind: "diff"; scope: "worktree" | "staged"; diff: GitFileDiff }
  | { kind: "content"; path: string; content: string; is_binary: boolean; truncated: boolean;
      reason: "ignored" | "unchanged" | "not_repository" }
  | { kind: "missing"; path: string };
```

Consumers: `preview?.kind === "diff"` in `src/components/git/GitDiffDialog.tsx`;
`GitPullState` (`status`) in `src/stores/gitStore.ts`; `switch (status)` in
`sidebarUpdateLabelKey` in `src/stores/updateStore.ts`.

---

## Validation of Untrusted Data

- **Type guards** `isXxx(value): value is Xxx` for string unions read from storage / input:
  `isThemeMode` (`src/lib/theme.ts`), `isCodeThemeId` (`src/lib/codeThemes.ts`),
  `isApiCallLogStatus` (`src/lib/apiLogs.ts`), `isAppLocale` (`src/lib/i18n/locale.ts`).
- **Hand-written coercers** for user JSON: `isRecord`, `asString`, `nullableString`,
  `asStringArray` in `src/lib/subagentJson.ts` — unknown fields degrade to defaults instead of
  throwing.
- **`JSON.parse` result** is typed `unknown` and checked, or cast and field-checked inside
  `try/catch` (`readExpanded` in `src/stores/workspaceStore.ts`, `readStored` in
  `src/stores/channelStore.ts`).
- **Errors** are `unknown`; convert with `errorMessage(error)` from `src/lib/toast.ts`
  (`error instanceof Error ? error.message : String(error)`). Tauri command errors arrive as
  plain strings from Rust, which is why the `String(error)` branch matters.

---

## Forbidden Patterns

| Don't | Do instead |
|-------|-----------|
| `any` (even though lint allows it) | `unknown` + guard, or a precise type |
| `enum` | string-literal union + optional `const` array + `isXxx` guard |
| Guessing DTO field casing | Read the Rust struct + its `serde` attributes |
| Duplicating a DTO interface in a component | Import it from `@/lib/types` |
| `invoke<Foo>(...)` inline in callers | A typed wrapper in `src/lib/backend.ts` |
| `// @ts-ignore` / `// @ts-expect-error` | Fix the type (zero occurrences in `src/`) |

Allowed but use sparingly: `as` casts after `JSON.parse` or `t(..., { returnObjects: true })`
(`DatabaseSection.tsx`), and non-null `!` when existence is guaranteed by a static table
(`GLOBAL_SHORTCUTS.find(...)!` in `src/hooks/useAppHotkeys.ts`).
