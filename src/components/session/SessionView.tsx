import { GitPanel } from "@/components/git/GitPanel";
import { useSessionStore } from "@/stores/sessionStore";
import { useUiStore } from "@/stores/uiStore";
import { Composer } from "./Composer";
import { EventStream } from "./EventStream";
import { SessionHeader } from "./SessionHeader";

export function SessionView() {
  const sessionId = useSessionStore((state) => state.selectedSessionId);
  const gitOpen = useUiStore((state) => state.gitOpen);
  if (!sessionId) return null;
  return (
    <div className="flex h-full min-h-0">
      <div className="flex min-w-0 flex-1 flex-col">
        <SessionHeader />
        <EventStream sessionId={sessionId} />
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
