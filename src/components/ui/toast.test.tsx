import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import {
  ToastItem,
  ToastOverflowButton,
  ToastProvider,
  TOAST_STACK_CLASS,
  TOAST_VIEWPORT_CLASS,
  Toaster,
  resolveVariant,
  type ToastItemProps,
} from "./toast";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: "zh-CN" },
  }),
}));

function makeToast(overrides: Partial<ToastItemProps["toast"]> = {}): ToastItemProps["toast"] {
  return {
    id: "toast-1",
    type: "success",
    title: "已保存",
    description: "设置已更新",
    ...overrides,
  };
}

function renderItem(toast = makeToast()): string {
  return renderToStaticMarkup(
    <ToastProvider>
      <ToastItem toast={toast} />
    </ToastProvider>,
  );
}

describe("resolveVariant", () => {
  it("keeps known variants", () => {
    expect(resolveVariant("success")).toBe("success");
    expect(resolveVariant("warning")).toBe("warning");
    expect(resolveVariant("error")).toBe("error");
    expect(resolveVariant("loading")).toBe("loading");
  });

  it("falls back to info for info, undefined and unknown types", () => {
    expect(resolveVariant("info")).toBe("info");
    expect(resolveVariant(undefined)).toBe("info");
    expect(resolveVariant("whatever")).toBe("info");
  });
});

describe("ToastItem SSR", () => {
  it("renders title, description and a close button", () => {
    const html = renderItem();

    expect(html).toContain("已保存");
    expect(html).toContain("设置已更新");
    expect(html).toContain('aria-label="close"');
  });

  it("dismisses only via the close button, never by clicking the card", () => {
    const html = renderItem();

    expect(html).not.toContain("cursor-pointer");
  });

  it("clamps long bodies and keeps them selectable", () => {
    const html = renderItem(makeToast({ description: Array(80).fill("长文本").join(" ") }));

    expect(html).toContain("overflow-y-auto");
    expect(html).toContain("select-text");
    expect(html).toContain("whitespace-pre-wrap");
  });

  it("sizes the body against the viewport height so short screens get shorter cards", () => {
    const html = renderItem(makeToast({ description: "长文本" }));

    expect(html).toContain("max-h-[min(30dvh,12rem)]");
  });

  it("styles each variant with its own icon and frame", () => {
    expect(renderItem(makeToast({ type: "success" }))).toContain("text-emerald-600");
    expect(renderItem(makeToast({ type: "warning" }))).toContain("text-amber-600");
    expect(renderItem(makeToast({ type: "error" }))).toContain("text-destructive");
    expect(renderItem(makeToast({ type: "loading" }))).toContain("animate-spin");
    expect(renderItem(makeToast({ type: undefined }))).toContain("text-muted-foreground");
  });

  it("omits the title/description nodes when the toast has none", () => {
    const html = renderItem(makeToast({ title: undefined, description: undefined }));

    expect(html).not.toContain("已保存");
    expect(html).not.toContain("overflow-y-auto");
    expect(html).toContain('aria-label="close"');
  });

  it("falls back to the notifications label so title-less dialogs keep an accessible name", () => {
    const titleless = renderItem(makeToast({ title: undefined, type: "error", priority: "high" }));

    expect(titleless).toContain('role="alertdialog"');
    expect(titleless).toContain('aria-label="notifications"');

    const titled = renderItem(makeToast({ title: "已保存" }));

    expect(titled).toContain('role="dialog"');
    expect(titled).not.toContain('aria-label="notifications"');
  });

  it("keeps limited toasts in the markup but hidden by CSS until expanded", () => {
    const html = renderItem(makeToast({ limited: true }));

    expect(html).toContain("data-limited");
    expect(html).toContain("data-limited:hidden");
  });
});

describe("ToastOverflowButton SSR", () => {
  it("renders a labelled button exposing how many notifications are collapsed", () => {
    const html = renderToStaticMarkup(<ToastOverflowButton hiddenCount={4} onExpand={() => {}} />);

    expect(html).toContain('type="button"');
    expect(html).toContain("toastMoreHidden");
  });
});

describe("toast stack layout classes", () => {
  it("keeps the outer viewport full-width and non-blocking, without clipping cards", () => {
    expect(TOAST_VIEWPORT_CLASS).toContain("pointer-events-none");
    expect(TOAST_VIEWPORT_CLASS).toContain("inset-x-0");
    expect(TOAST_VIEWPORT_CLASS).not.toContain("overflow-hidden");
  });

  it("puts the stack in a self-sized, scrollable and interactive container", () => {
    expect(TOAST_STACK_CLASS).toContain("pointer-events-auto");
    expect(TOAST_STACK_CLASS).toContain("w-fit");
    expect(TOAST_STACK_CLASS).toContain("max-h-[calc(100dvh-1.5rem)]");
    expect(TOAST_STACK_CLASS).toContain("overflow-y-auto");
    expect(TOAST_STACK_CLASS).toContain("overscroll-contain");
  });
});

describe("Toaster SSR", () => {
  it("renders nothing on the server because the portal mounts on the client", () => {
    const html = renderToStaticMarkup(
      <ToastProvider>
        <Toaster />
      </ToastProvider>,
    );

    expect(html).toBe("");
  });
});
