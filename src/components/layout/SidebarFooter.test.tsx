import { I18nextProvider } from "react-i18next";
import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { SidebarUpdateButton } from "./SidebarFooter";

function renderButton(status: "idle" | "available" | "downloading" | "ready") {
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

  it("shows 下载中 while the package is downloading", () => {
    expect(renderButton("downloading")).toContain("下载中");
  });

  it("shows 重启更新 after the package is installed", () => {
    expect(renderButton("ready")).toContain("重启更新");
  });
});
