import { I18nextProvider } from "react-i18next";
import { renderToString } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { beforeEach, describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { useUpdateStore } from "@/stores/updateStore";
import { SidebarFooter } from "./SidebarFooter";

const sampleUpdate = {
  version: "0.3.0",
  currentVersion: "0.2.3",
  notes: null,
  pubDate: null,
};

function renderFooter() {
  return renderToString(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>
        <SidebarFooter />
      </MemoryRouter>
    </I18nextProvider>,
  );
}

describe("SidebarFooter update button", () => {
  beforeEach(() => {
    useUpdateStore.setState({
      status: "idle",
      checking: false,
      startupChecked: false,
      update: null,
      progress: null,
      errorCode: null,
      errorDetail: "",
      relaunchFailedDetail: null,
      upToDate: false,
    });
  });

  it("hides the update button when no release is available", () => {
    const html = renderFooter();
    expect(html).toContain("noxcode");
    expect(html).not.toContain("更新");
    expect(html).not.toContain("下载中");
    expect(html).not.toContain("重启更新");
  });

  it("shows 更新 when a release is available", () => {
    useUpdateStore.setState({ status: "available", update: sampleUpdate });
    expect(renderFooter()).toContain("更新");
  });

  it("shows 下载中 while the package is downloading", () => {
    useUpdateStore.setState({ status: "downloading", update: sampleUpdate });
    expect(renderFooter()).toContain("下载中");
  });

  it("shows 重启更新 after the package is installed", () => {
    useUpdateStore.setState({ status: "ready", update: sampleUpdate });
    expect(renderFooter()).toContain("重启更新");
  });
});
