import { Send, Square } from "lucide-react";
import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  listNativeBackgroundTasks,
  sendNativeBackgroundMessage,
  stopNativeBackgroundTask,
} from "@/lib/backend";
import type { NativeBackgroundTask } from "@/lib/types";
import { useSessionStore } from "@/stores/sessionStore";
import { AssistantMarkdown } from "./AssistantMarkdown";

const STATUS = {
  queued: "排队中",
  running: "运行中",
  done: "已完成",
  failed: "失败",
  stopped: "已停止",
};

function BackgroundTaskRow({
  sessionId,
  task,
  live,
}: {
  sessionId: string;
  task: NativeBackgroundTask;
  live: boolean;
}) {
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const active = live && (task.status === "queued" || task.status === "running");
  const act = async (stop: boolean) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      if (stop) await stopNativeBackgroundTask(sessionId, task.task_id);
      else {
        await sendNativeBackgroundMessage(sessionId, task.task_id, message.trim());
        setMessage("");
      }
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  return (
    <details className="border-b py-2">
      <summary className="cursor-pointer break-words text-sm">
        {task.description || task.task_id}{" "}
        <span className="text-xs text-muted-foreground">{STATUS[task.status]}</span>
      </summary>
      <div className="mt-2 space-y-2">
        <p className="text-xs text-muted-foreground">
          {task.task_id} · {task.kind}
        </p>
        {task.report ? <AssistantMarkdown text={task.report} /> : null}
        {active ? (
          <form
            className="flex gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              if (message.trim()) void act(false);
            }}
          >
            <Input
              aria-label="后台任务消息"
              value={message}
              onChange={(event) => setMessage(event.target.value)}
              disabled={busy}
            />
            <Button
              size="icon"
              type="submit"
              title="发送消息"
              aria-label="发送消息"
              disabled={busy || !message.trim()}
            >
              <Send className="size-4" />
            </Button>
            <Button
              size="icon"
              type="button"
              variant="outline"
              title="停止后台任务"
              aria-label="停止后台任务"
              disabled={busy}
              onClick={() => {
                void act(true);
              }}
            >
              <Square className="size-4" />
            </Button>
          </form>
        ) : null}
        {error ? (
          <p role="alert" className="text-sm text-destructive">
            {error}
          </p>
        ) : null}
      </div>
    </details>
  );
}

export function BackgroundTasks({ sessionId }: { sessionId: string }) {
  const live = useSessionStore((state) => Boolean(state.liveBySession[sessionId]));
  const tasks = useSessionStore((state) => state.backgroundBySession[sessionId]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!live) return;
    let cancelled = false;
    const initial = useSessionStore.getState().backgroundBySession[sessionId];
    void listNativeBackgroundTasks(sessionId)
      .then((next) => {
        if (!cancelled && useSessionStore.getState().backgroundBySession[sessionId] === initial)
          useSessionStore
            .getState()
            .onBackgroundTasks({ session_record_id: sessionId, tasks: next });
      })
      .catch((reason) => {
        if (!cancelled) setError(String(reason));
      });
    return () => {
      cancelled = true;
    };
  }, [live, sessionId]);
  if (!tasks?.length && !error) return null;
  return (
    <section aria-label="后台任务">
      <h3 className="text-sm font-medium">后台任务</h3>
      {tasks?.map((task) => (
        <BackgroundTaskRow key={task.task_id} sessionId={sessionId} task={task} live={live} />
      ))}
      {error ? (
        <p role="alert" className="text-sm text-destructive">
          {error}
        </p>
      ) : null}
    </section>
  );
}
