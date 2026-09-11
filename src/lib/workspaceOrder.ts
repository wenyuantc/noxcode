export interface WorkspaceRowBounds {
  id: string;
  top: number;
  bottom: number;
}

/**
 * Reorders `items` to match the persisted `order` of ids.
 *
 * Ids missing from `items` (deleted workspaces) are dropped; items whose id is
 * not in `order` (newly created workspaces) keep their incoming order and are
 * appended after the known ones. Returns the input array when no reordering is
 * needed, so callers can compare references.
 */
export function applyWorkspaceOrder<T extends { id: string }>(items: T[], order: string[]): T[] {
  if (order.length === 0) return items;
  const rank = new Map(order.map((id, index) => [id, index]));
  const known = items
    .filter((item) => rank.has(item.id))
    .sort((left, right) => (rank.get(left.id) ?? 0) - (rank.get(right.id) ?? 0));
  if (known.length === items.length) {
    const unchanged = items.every((item, index) => item.id === known[index]?.id);
    return unchanged ? items : known;
  }
  const unknown = items.filter((item) => !rank.has(item.id));
  return [...known, ...unknown];
}

/**
 * Moves `id` so it lands at `insertionIndex`, where the index counts "insert
 * before this slot" positions in the current list (`0..items.length`).
 *
 * Returns the input array (same reference) when the item is unknown, the index
 * is not finite, or the move would leave the order untouched, so callers can
 * skip persisting a no-op.
 */
export function moveWorkspaceTo<T extends { id: string }>(
  items: T[],
  id: string,
  insertionIndex: number,
): T[] {
  const from = items.findIndex((item) => item.id === id);
  if (from < 0) return items;
  const requested = Number.isFinite(insertionIndex) ? insertionIndex : from;
  const clamped = Math.min(Math.max(requested, 0), items.length);
  // Dropping into either slot adjacent to its own position leaves the list as is.
  const to = clamped > from ? clamped - 1 : clamped;
  if (to === from) return items;
  const next = [...items];
  const [moved] = next.splice(from, 1);
  if (!moved) return items;
  next.splice(to, 0, moved);
  return next;
}

/**
 * Resolves the `insertionIndex` that moves `id` one slot in `direction`, for the
 * keyboard-reachable "move up / move down" menu entries.
 *
 * Returns `null` when the item is unknown or already at the requested boundary,
 * so callers can skip the move entirely.
 */
export function workspaceMoveTarget<T extends { id: string }>(
  items: T[],
  id: string,
  direction: "up" | "down",
): number | null {
  const index = items.findIndex((item) => item.id === id);
  if (index < 0) return null;
  if (direction === "up") return index > 0 ? index - 1 : null;
  return index < items.length - 1 ? index + 2 : null;
}

/**
 * Resolves the insertion index (`0..rows.length`) for a pointer at `pointerY`.
 *
 * Each row contributes its upper half to "insert before it" and its lower half
 * to "insert after it"; a pointer above the first row inserts at the top and a
 * pointer below the last row inserts at the bottom.
 */
export function workspaceDropIndex(rows: WorkspaceRowBounds[], pointerY: number): number {
  for (let index = 0; index < rows.length; index += 1) {
    const row = rows[index];
    if (!row) continue;
    if (pointerY < row.top) return index;
    if (pointerY <= row.bottom) {
      const middle = row.top + (row.bottom - row.top) / 2;
      return pointerY <= middle ? index : index + 1;
    }
  }
  return rows.length;
}

/**
 * Offset of the drop indicator inside a scroll container, in content-box pixels.
 *
 * The indicator is drawn above the row at `index`, or below the last block when
 * `index` points past the end (`tailBottom` is that block's bottom edge, so a
 * trailing drop lands after the block's sessions rather than between its header
 * and its session list). Both `rows` and `containerTop` are expected in viewport
 * coordinates, so a scrolled list resolves to the same visual line.
 */
export function workspaceIndicatorOffset(options: {
  rows: WorkspaceRowBounds[];
  index: number;
  containerTop: number;
  scrollTop: number;
  tailBottom?: number;
}): number | null {
  const { rows, index, containerTop, scrollTop, tailBottom } = options;
  if (rows.length === 0) return null;
  const anchor = index >= rows.length ? rows[rows.length - 1] : rows[index];
  if (!anchor) return null;
  const viewportY = index >= rows.length ? (tailBottom ?? anchor.bottom) : anchor.top;
  return viewportY - containerTop + scrollTop;
}
