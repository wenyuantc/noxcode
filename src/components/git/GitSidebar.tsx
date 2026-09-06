import { useEffect, useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";

import { GIT_PANEL_DEFAULT_WIDTH, gitPanelLayout } from "@/lib/gitPanelLayout";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/stores/uiStore";
import { GitPanel } from "./GitPanel";

export function GitSidebar({ containerRef }: { containerRef: RefObject<HTMLDivElement | null> }) {
  const { t } = useTranslation("git");
  const preferredWidth = useUiStore((state) => state.gitPanelWidth);
  const setWidth = useUiStore((state) => state.setGitPanelWidth);
  const [containerWidth, setContainerWidth] = useState(0);
  const drag = useRef<{ x: number; width: number } | null>(null);
  const cleanupDrag = useRef<(() => void) | null>(null);
  const { width, minWidth, maxWidth, overlay } = gitPanelLayout(containerWidth, preferredWidth);

  // The parent ref is not attached yet during a child's first layout effect.
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const measure = () => setContainerWidth(container.getBoundingClientRect().width);
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    measure();
    const stop = () => cleanupDrag.current?.();
    window.addEventListener("blur", stop);
    return () => {
      observer.disconnect();
      window.removeEventListener("blur", stop);
      stop();
    };
  }, [containerRef]);

  const resize = (next: number) => setWidth(Math.min(maxWidth, Math.max(minWidth, next)));

  return (
    <>
      {overlay && containerWidth > 0 ? (
        <button
          type="button"
          tabIndex={-1}
          aria-label={t("close")}
          className="absolute inset-0 z-20 bg-black/10"
          onClick={() => useUiStore.getState().toggleGit()}
        />
      ) : null}
      <aside
        aria-label={t("panel")}
        className={cn(
          "flex min-h-0 shrink-0 bg-background",
          overlay && "absolute inset-y-0 right-0 z-30 shadow-xl",
          containerWidth === 0 && "invisible",
        )}
      >
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label={t("resizePanel")}
          aria-controls="git-sidebar-content"
          aria-valuemin={minWidth}
          aria-valuemax={maxWidth}
          aria-valuenow={width}
          tabIndex={0}
          className="w-1 shrink-0 touch-none cursor-col-resize border-l border-border hover:bg-ring/40 focus-visible:bg-ring/40 focus-visible:outline-none"
          onDoubleClick={() => setWidth(GIT_PANEL_DEFAULT_WIDTH)}
          onKeyDown={(event) => {
            const next = {
              ArrowLeft: width + 16,
              ArrowRight: width - 16,
              Home: minWidth,
              End: maxWidth,
            }[event.key];
            if (next === undefined) return;
            event.preventDefault();
            resize(next);
          }}
          onPointerDown={(event) => {
            if (event.button !== 0) return;
            event.preventDefault();
            cleanupDrag.current?.();
            const handle = event.currentTarget;
            const pointerId = event.pointerId;
            handle.setPointerCapture(pointerId);
            handle.focus();
            drag.current = { x: event.clientX, width };
            const { userSelect, cursor } = document.body.style;
            document.body.style.userSelect = "none";
            document.body.style.cursor = "col-resize";
            cleanupDrag.current = () => {
              drag.current = null;
              cleanupDrag.current = null;
              document.body.style.userSelect = userSelect;
              document.body.style.cursor = cursor;
              if (handle.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId);
            };
          }}
          onPointerMove={(event) => {
            if (drag.current) resize(drag.current.width + drag.current.x - event.clientX);
          }}
          onPointerUp={() => cleanupDrag.current?.()}
          onPointerCancel={() => cleanupDrag.current?.()}
          onLostPointerCapture={() => cleanupDrag.current?.()}
        />
        <div id="git-sidebar-content" className="min-h-0 min-w-0" style={{ width }}>
          <GitPanel />
        </div>
      </aside>
    </>
  );
}
