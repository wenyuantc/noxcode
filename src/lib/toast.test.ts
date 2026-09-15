import { afterEach, describe, expect, it } from "vitest";

import {
  limitedToastCount,
  TOAST_LIMIT,
  TOAST_LIMIT_EXPANDED,
  TOAST_TIMEOUT_MS,
  dismissToast,
  errorMessage,
  runToastAction,
  showToast,
  toastAddOptions,
  toastManager,
  toastPriorityFor,
  toastTimeoutFor,
  toastUpdateOptions,
  updateToast,
} from "./toast";

interface ManagerEvent {
  action: "add" | "close" | "update" | "promise";
  options: Record<string, unknown>;
}

/**
 * base-ui 把订阅入口暴露为带前导空格的私有键（`' subscribe'`），
 * 这里集中封装，测试只依赖 manager 的事件流。
 */
function subscribeToasts(listener: (event: ManagerEvent) => void): () => void {
  const manager = toastManager as unknown as {
    " subscribe": (listener: (event: ManagerEvent) => void) => () => void;
  };
  return manager[" subscribe"](listener);
}

function collectEvents() {
  const events: ManagerEvent[] = [];
  const unsubscribe = subscribeToasts((event) => events.push(event));
  return { events, unsubscribe };
}

const cleanups: Array<() => void> = [];

afterEach(() => {
  while (cleanups.length) cleanups.pop()?.();
  dismissToast();
});

describe("toastAddOptions", () => {
  it("defaults to info with the variant timeout and polite priority", () => {
    expect(toastAddOptions({ description: "已保存" })).toEqual({
      id: undefined,
      type: "info",
      title: undefined,
      description: "已保存",
      timeout: TOAST_TIMEOUT_MS.info,
      priority: "low",
    });
  });

  it("keeps error toasts open and announces them urgently", () => {
    expect(toastAddOptions({ variant: "error", description: "失败了" })).toMatchObject({
      type: "error",
      timeout: 0,
      priority: "high",
    });
  });

  it("honours an explicit timeout and id", () => {
    expect(
      toastAddOptions({ id: "lsp-test-rust", variant: "loading", timeout: 1200, title: "测试中" }),
    ).toMatchObject({
      id: "lsp-test-rust",
      type: "loading",
      title: "测试中",
      timeout: 1200,
      priority: "low",
    });
  });
});

describe("toast variant helpers", () => {
  it("maps every variant to its timeout", () => {
    expect(toastTimeoutFor("success")).toBe(3000);
    expect(toastTimeoutFor("warning")).toBe(8000);
    expect(toastTimeoutFor("error")).toBe(0);
    expect(toastTimeoutFor("loading")).toBe(0);
  });

  it("only error is high priority", () => {
    expect(toastPriorityFor("error")).toBe("high");
    for (const variant of ["success", "info", "warning", "loading"] as const) {
      expect(toastPriorityFor(variant)).toBe("low");
    }
  });

  it("formats unknown failures as text", () => {
    expect(errorMessage(new Error("boom"))).toBe("boom");
    expect(errorMessage("plain")).toBe("plain");
  });
});

describe("limitedToastCount", () => {
  it("is zero when nothing is collapsed", () => {
    expect(limitedToastCount([])).toBe(0);
    expect(limitedToastCount([{ limited: false }, { transitionStatus: "starting" }])).toBe(0);
  });

  it("counts collapsed toasts so the stack can advertise them", () => {
    expect(
      limitedToastCount([
        { limited: true },
        { limited: false },
        { limited: true, transitionStatus: "starting" },
      ]),
    ).toBe(2);
  });

  it("ignores collapsed toasts that are already animating out", () => {
    expect(
      limitedToastCount([
        { limited: true, transitionStatus: "ending" },
        { limited: true, transitionStatus: "starting" },
      ]),
    ).toBe(1);
  });

  it("keeps the default stack limit and only widens it on expand", () => {
    expect(TOAST_LIMIT).toBe(3);
    expect(TOAST_LIMIT_EXPANDED).toBeGreaterThan(TOAST_LIMIT);
    expect(limitedToastCount(Array.from({ length: 50 }, () => ({ limited: true })))).toBe(50);
  });
});

describe("toastUpdateOptions", () => {
  it("keeps unspecified fields untouched (sparse patch)", () => {
    expect(toastUpdateOptions({ description: "完成" })).toEqual({ description: "完成" });
  });

  it("refreshes priority and timer when the variant changes", () => {
    expect(toastUpdateOptions({ variant: "success" })).toEqual({
      type: "success",
      priority: "low",
      timeout: 3000,
    });
  });

  it("lets an explicit timeout win over the variant default", () => {
    expect(toastUpdateOptions({ variant: "warning", timeout: 500 })).toEqual({
      type: "warning",
      priority: "low",
      timeout: 500,
    });
  });
});

describe("toast manager API", () => {
  it("showToast returns the id and emits an add event", () => {
    const { events, unsubscribe } = collectEvents();
    cleanups.push(unsubscribe);

    const id = showToast({ id: "fixed-id", variant: "success", description: "已保存" });

    expect(id).toBe("fixed-id");
    expect(events).toHaveLength(1);
    expect(events[0].action).toBe("add");
    expect(events[0].options).toMatchObject({
      id: "fixed-id",
      type: "success",
      timeout: 3000,
      priority: "low",
    });
  });

  it("updateToast patches only the given fields of the same id", () => {
    const { events, unsubscribe } = collectEvents();
    cleanups.push(unsubscribe);

    showToast({ id: "lsp-test-rust", variant: "loading", title: "测试中" });
    updateToast("lsp-test-rust", { variant: "success", description: "rust-analyzer · 820 ms" });

    expect(events.map((event) => event.action)).toEqual(["add", "update"]);
    expect(events[1].options).toEqual({
      id: "lsp-test-rust",
      type: "success",
      priority: "low",
      timeout: 3000,
      description: "rust-analyzer · 820 ms",
    });
  });

  it("dismissToast closes one id, or everything without an id", () => {
    const { events, unsubscribe } = collectEvents();
    cleanups.push(unsubscribe);

    dismissToast("toast-1");
    dismissToast();

    expect(events).toEqual([
      { action: "close", options: { id: "toast-1" } },
      { action: "close", options: { id: undefined } },
    ]);
  });

  it("re-adding a closed id emits another add event instead of being swallowed", () => {
    const { events, unsubscribe } = collectEvents();
    cleanups.push(unsubscribe);

    showToast({ id: "same-id", variant: "error", description: "第一次失败" });
    dismissToast("same-id");
    showToast({ id: "same-id", variant: "error", description: "重入后的失败" });

    // manager 不做去重、也不吞掉重入的 add；Provider 的 store 收到 add 后会把
    // 「正在退场」的同 id 通知移除再重新挂载，所以第二条消息不会丢。
    expect(events.map((event) => event.action)).toEqual(["add", "close", "add"]);
    expect(events[2].options).toMatchObject({
      id: "same-id",
      type: "error",
      description: "重入后的失败",
    });
  });
});

describe("runToastAction", () => {
  it("stays silent on success unless a message is given", async () => {
    const { events, unsubscribe } = collectEvents();
    cleanups.push(unsubscribe);

    await expect(runToastAction(async () => undefined)).resolves.toBe(true);
    expect(events).toHaveLength(0);

    await expect(
      runToastAction(async () => undefined, { successMessage: "已复制", id: "copy" }),
    ).resolves.toBe(true);
    expect(events).toHaveLength(1);
    expect(events[0].options).toMatchObject({
      id: "copy",
      type: "success",
      description: "已复制",
    });
  });

  it("reports failures as a non-dismissing error toast", async () => {
    const { events, unsubscribe } = collectEvents();
    cleanups.push(unsubscribe);

    await expect(
      runToastAction(async () => {
        throw new Error("目录不存在");
      }),
    ).resolves.toBe(false);

    expect(events).toHaveLength(1);
    expect(events[0].options).toMatchObject({
      type: "error",
      description: "目录不存在",
      timeout: 0,
      priority: "high",
    });
  });
});
