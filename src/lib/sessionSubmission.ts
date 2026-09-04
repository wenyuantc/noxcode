import { resumeNativeSession, startNativeSession } from "@/lib/backend";
import type { StartNativeSessionInput } from "@/lib/types";

export interface SessionSubmissionInput {
  sessionId?: string | null;
  workspaceId: string;
  channelId: string;
  prompt: string;
  model?: string | null;
  reasoningEffort?: string | null;
  planMode?: boolean | null;
  imagePaths?: string[] | null;
}

export interface SessionSubmissionApi {
  startNativeSession: (payload: StartNativeSessionInput) => Promise<unknown>;
  resumeNativeSession: (
    payload: StartNativeSessionInput,
    resumeSessionId?: string,
  ) => Promise<unknown>;
}

const defaultApi: SessionSubmissionApi = {
  startNativeSession,
  resumeNativeSession,
};

export function sessionSubmissionPayload(input: SessionSubmissionInput): StartNativeSessionInput {
  const sessionId = input.sessionId?.trim() || null;
  const imagePaths = (input.imagePaths ?? [])
    .map((path) => path.trim())
    .filter((path) => path.length > 0);
  return {
    ai_channel_id: input.channelId,
    workspace_id: input.workspaceId,
    prompt: input.prompt,
    model: input.model ?? null,
    reasoning_effort: input.reasoningEffort ?? null,
    plan_mode: input.planMode ?? null,
    resume_session_id: sessionId,
    image_paths: imagePaths.length > 0 ? imagePaths : null,
  };
}

export async function submitSessionPrompt(
  input: SessionSubmissionInput,
  api: SessionSubmissionApi = defaultApi,
): Promise<void> {
  const payload = sessionSubmissionPayload(input);
  const sessionId = payload.resume_session_id;
  if (sessionId) {
    await api.resumeNativeSession(payload, sessionId);
    return;
  }
  await api.startNativeSession(payload);
}
