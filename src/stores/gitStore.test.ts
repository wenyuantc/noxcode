import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/backend", () => ({ pullGitBranch: vi.fn() }));

import { pullGitBranch } from "@/lib/backend";
import type { GitPullResult } from "@/lib/types";
import { useGitStore } from "./gitStore";

function deferred() {
  let resolve!: (value: GitPullResult) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<GitPullResult>((accept, fail) => {
    resolve = accept;
    reject = fail;
  });
  return { promise, resolve, reject };
}

beforeEach(() => {
  vi.mocked(pullGitBranch).mockReset();
  useGitStore.setState({ pulls: {} });
});

describe("Git pulls", () => {
  it("deduplicates requests even when there are no mounted subscribers", async () => {
    const request = deferred();
    vi.mocked(pullGitBranch).mockReturnValue(request.promise);
    const unsubscribe = useGitStore.subscribe(() => undefined);
    const pending = useGitStore.getState().pull("workspace-a");
    unsubscribe();
    await useGitStore.getState().pull("workspace-a");
    expect(pullGitBranch).toHaveBeenCalledTimes(1);
    expect(useGitStore.getState().pulls["workspace-a"]?.status).toBe("pulling");
    request.resolve({ updated: true, message: "Fast-forward" });
    await pending;
    expect(useGitStore.getState().pulls["workspace-a"]).toEqual({
      status: "success",
      result: { updated: true, message: "Fast-forward" },
    });
  });

  it("keeps late results and errors scoped to the originating workspace", async () => {
    const a = deferred();
    const b = deferred();
    vi.mocked(pullGitBranch).mockReturnValueOnce(a.promise).mockReturnValueOnce(b.promise);
    const first = useGitStore.getState().pull("workspace-a");
    const second = useGitStore.getState().pull("workspace-b");
    b.resolve({ updated: false, message: "Already up to date" });
    await second;
    a.reject(new Error("authentication failed"));
    await first;
    expect(useGitStore.getState().pulls["workspace-a"]).toEqual({
      status: "error",
      error: "authentication failed",
    });
    expect(useGitStore.getState().pulls["workspace-b"]).toEqual({
      status: "success",
      result: { updated: false, message: "Already up to date" },
    });
  });

  it("reports IPC string errors and permits retry without an unhandled rejection", async () => {
    vi.mocked(pullGitBranch)
      .mockRejectedValueOnce("cannot fast-forward")
      .mockResolvedValueOnce({ updated: false, message: "Already up to date" });
    await useGitStore.getState().pull("workspace-a");
    expect(useGitStore.getState().pulls["workspace-a"]).toEqual({
      status: "error",
      error: "cannot fast-forward",
    });
    await useGitStore.getState().pull("workspace-a");
    expect(pullGitBranch).toHaveBeenCalledTimes(2);
    expect(useGitStore.getState().pulls["workspace-a"]?.status).toBe("success");
  });
});
