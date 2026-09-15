import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";

import {
  workspaceDropIndex,
  workspaceIndicatorOffset,
  type WorkspaceRowBounds,
} from "@/lib/workspaceOrder";

/** Vertical movement in pixels before a press turns into a drag instead of a click. */
const DRAG_THRESHOLD = 4;

/** Presses inside the hover action area must not start a row drag. */
const NO_DRAG_SELECTOR = "[data-no-drag]";

interface DragState {
  id: string;
  pointerId: number;
  startY: number;
  active: boolean;
  index: number;
  scopeId: string | null;
  restoreBody: (() => void) | null;
}

export type PointerRowProps = {
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
  onPointerMove: (event: ReactPointerEvent<HTMLElement>) => void;
  onPointerUp: (event: ReactPointerEvent<HTMLElement>) => void;
  onPointerCancel: (event: ReactPointerEvent<HTMLElement>) => void;
  onLostPointerCapture: (event: ReactPointerEvent<HTMLElement>) => void;
  "data-workspace-row"?: string;
  "data-session-row"?: string;
};

export interface PointerReorder {
  /** Id of the row being dragged, or `null` when idle. */
  draggingId: string | null;
  /** Y offset in pixels, relative to the scroll container, for the drop indicator. */
  indicatorY: number | null;
  /** Spread onto each row; the row element must keep `data-${rowAttr}={id}`. */
  rowProps: (id: string) => PointerRowProps;
}

export interface WorkspaceDrag extends PointerReorder {
  rowProps: (id: string) => PointerRowProps & { "data-workspace-row": string };
}

function datasetCamel(attr: string): string {
  return attr.replace(/-([a-z])/g, (_, letter: string) => letter.toUpperCase());
}

function dataValue(element: HTMLElement, attr: string): string | undefined {
  return element.dataset[datasetCamel(attr)];
}

/** Measures rows in viewport coordinates; re-read on every move so scrolling stays in sync. */
function readRows(
  container: HTMLElement | null,
  rowAttr: string,
  scopeAttr: string | undefined,
  scopeId: string | null,
): WorkspaceRowBounds[] {
  if (!container) return [];
  const root =
    scopeAttr && scopeId
      ? container.querySelector<HTMLElement>(`[data-${scopeAttr}="${CSS.escape(scopeId)}"]`)
      : container;
  if (!root) return [];
  return Array.from(root.querySelectorAll<HTMLElement>(`[data-${rowAttr}]`)).flatMap((element) => {
    const id = dataValue(element, rowAttr);
    if (!id) return [];
    const rect = element.getBoundingClientRect();
    return [{ id, top: rect.top, bottom: rect.bottom }];
  });
}

/** Bottom edge of the last matching node, used to anchor a trailing drop. */
function readTailBottom(
  container: HTMLElement | null,
  tailSelector: string | undefined,
): number | undefined {
  if (!container || !tailSelector) return undefined;
  const blocks = container.querySelectorAll<HTMLElement>(tailSelector);
  const last = blocks[blocks.length - 1];
  return last?.getBoundingClientRect().bottom;
}

/** Resolves the indicator offset for a resolved drop index. */
function indicatorFor(
  container: HTMLElement | null,
  rows: WorkspaceRowBounds[],
  index: number,
  tailSelector: string | undefined,
): number | null {
  if (!container) return null;
  return workspaceIndicatorOffset({
    rows,
    index,
    containerTop: container.getBoundingClientRect().top,
    scrollTop: container.scrollTop,
    tailBottom: readTailBottom(container, tailSelector),
  });
}

/**
 * Pointer-driven reordering for a vertical list of rows.
 *
 * A press only becomes a drag after {@link DRAG_THRESHOLD} pixels of vertical
 * movement, which keeps plain clicks working; pointer capture is taken at that
 * moment instead of on press, so the click target stays the row's own button.
 * The click that follows a drag is swallowed once, so dropping a row never
 * triggers the row action. When `scopeAttr` is set, only rows inside the
 * dragged item's ancestor with that data attribute are measured.
 */
export function usePointerReorder(options: {
  containerRef: RefObject<HTMLElement | null>;
  onMove: (id: string, insertionIndex: number) => void;
  rowAttr: "workspace-row" | "session-row";
  scopeAttr?: "workspace-block";
  tailSelector?: string;
}): PointerReorder {
  const { containerRef, onMove, rowAttr, scopeAttr, tailSelector } = options;
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [indicatorY, setIndicatorY] = useState<number | null>(null);
  const state = useRef<DragState | null>(null);
  const suppressClick = useRef(false);
  const onMoveRef = useRef(onMove);
  onMoveRef.current = onMove;

  const finish = useCallback((commit: boolean) => {
    const current = state.current;
    state.current = null;
    if (!current) return;
    current.restoreBody?.();
    setDraggingId(null);
    setIndicatorY(null);
    if (commit && current.active) onMoveRef.current(current.id, current.index);
  }, []);

  useEffect(() => {
    const stop = () => finish(false);
    window.addEventListener("blur", stop);
    return () => {
      window.removeEventListener("blur", stop);
      finish(false);
    };
  }, [finish]);

  // A drag ends with a click on the row underneath the pointer; swallow that one
  // click. The flag also expires on the next tick, so a gesture that never
  // produced a click (released outside the window, for example) cannot eat a
  // later, unrelated click.
  useEffect(() => {
    const onClick = (event: MouseEvent) => {
      if (!suppressClick.current) return;
      suppressClick.current = false;
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener("click", onClick, true);
    return () => window.removeEventListener("click", onClick, true);
  }, []);

  const armClickSuppressor = useCallback(() => {
    suppressClick.current = true;
    window.setTimeout(() => {
      suppressClick.current = false;
    }, 0);
  }, []);

  const rowProps = useCallback(
    (id: string): PointerRowProps => ({
      ...(rowAttr === "workspace-row" ? { "data-workspace-row": id } : { "data-session-row": id }),
      onPointerDown: (event: ReactPointerEvent<HTMLElement>) => {
        if (event.button !== 0) return;
        const target = event.target;
        if (target instanceof Element && target.closest(NO_DRAG_SELECTOR)) return;
        // A second press while a gesture is pending must not leak its body styles.
        finish(false);
        const scope = scopeAttr
          ? event.currentTarget.closest<HTMLElement>(`[data-${scopeAttr}]`)
          : null;
        const scopeId = scope && scopeAttr ? (dataValue(scope, scopeAttr) ?? null) : null;
        if (scopeAttr && !scopeId) return;
        const rows = readRows(containerRef.current, rowAttr, scopeAttr, scopeId);
        if (!rows.some((row) => row.id === id)) return;
        state.current = {
          id,
          pointerId: event.pointerId,
          startY: event.clientY,
          active: false,
          index: rows.findIndex((row) => row.id === id),
          scopeId,
          restoreBody: null,
        };
      },
      onPointerMove: (event: ReactPointerEvent<HTMLElement>) => {
        const current = state.current;
        if (!current || current.pointerId !== event.pointerId) return;
        if (!current.active) {
          if (Math.abs(event.clientY - current.startY) < DRAG_THRESHOLD) return;
          current.active = true;
          setDraggingId(current.id);
          try {
            event.currentTarget.setPointerCapture(event.pointerId);
          } catch {
            // Capture is best-effort: without it the gesture still ends on pointerup.
          }
          event.preventDefault();
          const { userSelect, cursor } = document.body.style;
          current.restoreBody = () => {
            document.body.style.userSelect = userSelect;
            document.body.style.cursor = cursor;
          };
          document.body.style.userSelect = "none";
          document.body.style.cursor = "grabbing";
        }
        const container = containerRef.current;
        const rows = readRows(container, rowAttr, scopeAttr, current.scopeId);
        current.index = workspaceDropIndex(rows, event.clientY);
        setIndicatorY(indicatorFor(container, rows, current.index, tailSelector));
      },
      onPointerUp: (event: ReactPointerEvent<HTMLElement>) => {
        const current = state.current;
        if (!current || current.pointerId !== event.pointerId || event.button !== 0) return;
        // Re-resolve against the release position so a scroll mid-gesture cannot
        // commit a stale index.
        if (current.active) {
          current.index = workspaceDropIndex(
            readRows(containerRef.current, rowAttr, scopeAttr, current.scopeId),
            event.clientY,
          );
          armClickSuppressor();
        }
        finish(true);
      },
      onPointerCancel: (event: ReactPointerEvent<HTMLElement>) => {
        const current = state.current;
        if (current && current.pointerId !== event.pointerId) return;
        // pointercancel never produces a click, so it must not arm the suppressor.
        finish(false);
      },
      // Reached without a preceding pointerup only when capture is lost mid-gesture.
      onLostPointerCapture: (event: ReactPointerEvent<HTMLElement>) => {
        const current = state.current;
        if (current && current.pointerId !== event.pointerId) return;
        finish(false);
      },
    }),
    [armClickSuppressor, containerRef, finish, rowAttr, scopeAttr, tailSelector],
  );

  return { draggingId, indicatorY, rowProps };
}

/**
 * Pointer-driven reordering for the workspace list.
 *
 * A press only becomes a drag after {@link DRAG_THRESHOLD} pixels of vertical
 * movement, which keeps plain clicks (expand / activate) working; pointer
 * capture is taken at that moment instead of on press, so the click target
 * stays the row's own button. The click that follows a drag is swallowed once,
 * so dropping a row never toggles its expansion.
 */
export function useWorkspaceDrag(options: {
  containerRef: RefObject<HTMLElement | null>;
  onMove: (id: string, insertionIndex: number) => void;
}): WorkspaceDrag {
  return usePointerReorder({
    containerRef: options.containerRef,
    onMove: options.onMove,
    rowAttr: "workspace-row",
    tailSelector: "[data-workspace-block]",
  }) as WorkspaceDrag;
}
