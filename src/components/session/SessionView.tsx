import { useRef } from "react";

import { GitPanel } from "@/components/git/GitPanel";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { Composer } from "./Composer";
import { EventStream } from "./EventStream";
import { SessionHeader } from "./SessionHeader";
import { TodoProcessPanel } from "./TodoProcessPanel";

export function SessionView() {
  const sessionId = useSessionStore((state) => state.selectedSessionId);
  const gitOpen = useUiStore((state) => state.gitOpen);
  const streamRef = useRef<HTMLDivElement>(null);
  if (!sessionId) return null;
  return (
    <div className="flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <SessionHeader />
        <div ref={streamRef} className="relative min-h-0 flex-1">
          <EventStream sessionId={sessionId} />
          <TodoProcessPanel sessionId={sessionId} containerRef={streamRef} />
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
