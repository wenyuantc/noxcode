import { Loader2 } from "lucide-react";
import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { GitPanel } from "@/components/git/GitPanel";
import { cn } from "@/lib/utils";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { Composer } from "./Composer";
import { EventStream } from "./EventStream";
import { SessionHeader } from "./SessionHeader";
import { TodoProcessPanel } from "./TodoProcessPanel";

const KEEP_ALIVE = 3;

function cachedSessionIds(sessionId: string | null): string[] {
  if (!sessionId || !useSessionStore.getState().lines[sessionId]) return [];
  return [sessionId];
}

function rememberSession(ids: string[], sessionId: string): string[] {
  return [sessionId, ...ids.filter((id) => id !== sessionId)].slice(0, KEEP_ALIVE);
}

export function SessionView() {
  const { t } = useTranslation("common");
  const sessionId = useSessionStore((state) => state.selectedSessionId);
  const hasSelectedLines = useSessionStore((state) =>
    sessionId ? Boolean(state.lines[sessionId]) : false,
  );
  const gitOpen = useUiStore((state) => state.gitOpen);
  const streamRef = useRef<HTMLDivElement>(null);
  const [viewedId, setViewedId] = useState(() => cachedSessionIds(sessionId)[0] ?? null);
  const [mountedIds, setMountedIds] = useState(() => cachedSessionIds(sessionId));
  const historyReady = Boolean(sessionId && hasSelectedLines);

  if (sessionId && hasSelectedLines && viewedId !== sessionId) {
    setViewedId(sessionId);
    setMountedIds((ids) => rememberSession(ids, sessionId));
  }

  if (!sessionId) return null;
  return (
    <div className="flex h-full min-h-0">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <SessionHeader />
        <div ref={streamRef} className="relative min-h-0 flex-1">
          {historyReady ? (
            mountedIds.map((id) => (
              <div key={id} className={cn("h-full min-h-0", id !== viewedId && "hidden")}>
                <EventStream sessionId={id} active={id === viewedId} />
              </div>
            ))
          ) : (
            <div className="flex h-full items-center justify-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="size-4 animate-spin" />
              {t("loading")}
            </div>
          )}
          {historyReady && viewedId ? (
            <TodoProcessPanel sessionId={viewedId} containerRef={streamRef} />
          ) : null}
        </div>
        <div className="border-t px-4 py-3">
          <Composer compact />
        </div>
      </div>
      {gitOpen ? (
        <div className="w-[380px] shrink-0 border-l">
          <GitPanel />
        </div>
      ) : null}
    </div>
  );
}
