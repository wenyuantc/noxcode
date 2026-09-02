/**
 * Frontend SQL access is intentionally closed.
 * All business reads and writes must go through Tauri commands via `@/lib/backend`.
 */

const CLOSED_MESSAGE = "前端禁止直接访问 SQL。请改用后端 Tauri command（见 @/lib/backend）。";

/**
 * @deprecated Frontend database access is blocked. Use Tauri commands.
 */
export async function getDb(): Promise<never> {
  throw new Error(CLOSED_MESSAGE);
}

/**
 * @deprecated Frontend SQL select is blocked. Use Tauri list/get commands via backend.ts.
 */
export async function select<T>(_query: string, _params?: unknown[]): Promise<T[]> {
  throw new Error(CLOSED_MESSAGE);
}

/**
 * @deprecated Frontend writes are blocked by capabilities (no sql:allow-execute).
 * Use Tauri commands for all mutations.
 */
export async function execute(_query: string, _params?: unknown[]): Promise<void> {
  throw new Error("前端禁止直接执行写 SQL。请改用后端 Tauri command。");
}
