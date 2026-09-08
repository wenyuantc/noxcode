import { I18nextProvider } from "react-i18next";
import { renderToString } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import i18n from "@/lib/i18n";
import { AboutSection } from "@/components/settings/AboutSection";
import { useUpdateStore, type AppUpdateStatus } from "@/stores/updateStore";
import { SidebarUpdateButton } from "./SidebarFooter";

// Server rendering uses Zustand's initial snapshot; these views need the current test state.
vi.mock("@/stores/updateStore", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/stores/updateStore")>();
  return {
    ...actual,
    useUpdateStore: Object.assign(
      <T,>(selector: (state: ReturnType<typeof actual.useUpdateStore.getState>) => T) =>
        selector(actual.useUpdateStore.getState()),
      actual.useUpdateStore,
    ),
  };
});

function renderButton(status: AppUpdateStatus) {
  return renderToString(
    <I18nextProvider i18n={i18n}>
      <SidebarUpdateButton
        status={status}
        version="0.3.0"
        onDownload={() => undefined}
        onRelaunch={() => undefined}
      />
    </I18nextProvider>,
  );
}

describe("SidebarUpdateButton", () => {
  it("renders nothing when no release is available", () => {
    expect(renderButton("idle")).toBe("");
  });

  it("shows 更新 when a release is available", () => {
    const html = renderButton("available");
    expect(html).toContain("更新");
    expect(html).toContain("更新到 0.3.0");
  });

  it("shows 下载中 while the package is downloading without known percent", () => {
    expect(renderButton("downloading")).toContain("下载中");
  });

  it("shows download percentage while downloading when percent is known", () => {
    const html = renderToString(
      <I18nextProvider i18n={i18n}>
        <SidebarUpdateButton
          status="downloading"
          version="0.3.0"
          progress={{ downloaded: 17, total: 100, percent: 17 }}
          onDownload={() => undefined}
          onRelaunch={() => undefined}
        />
      </I18nextProvider>,
    );
    expect(html).toContain("17%");
    expect(html).toContain("bg-blue-600");
  });

  it("shows 重启更新 after the package is installed", () => {
    expect(renderButton("ready")).toContain("重启更新");
  });

  it("disables both update entry points and displays the shared restart state", () => {
    const previous = useUpdateStore.getState();
    useUpdateStore.setState({
      status: "restarting",
      update: { version: "0.3.5", currentVersion: "0.3.4", notes: null, pubDate: null },
    });
    try {
      const sidebar = renderButton(useUpdateStore.getState().status);
      const about = renderToString(
        <I18nextProvider i18n={i18n}>
          <AboutSection />
        </I18nextProvider>,
      );
      for (const html of [sidebar, about]) {
        expect(html).toContain("正在重启");
        expect(html).toContain('disabled=""');
        expect(html).toContain("animate-spin");
      }
      expect(about).not.toContain("立即重启");
    } finally {
      useUpdateStore.setState(previous);
    }
  });
});
