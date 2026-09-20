import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { packageAppVersion } from "@/lib/appUpdate";

import { SettingsBrandFooter, SETTINGS_NAV_GROUPS } from "./SettingsLayout";

describe("settings navigation", () => {
  it("places computer control next to LSP in the agent group", () => {
    const agent = SETTINGS_NAV_GROUPS.find((group) => group.id === "agent");
    expect(agent?.items).toContain("computer");
    expect(agent?.items.indexOf("computer")).toBe((agent?.items.indexOf("lsp") ?? -1) + 1);
  });
});

describe("SettingsBrandFooter", () => {
  it("renders the current package version instead of a stale hardcoded label", () => {
    const html = renderToString(<SettingsBrandFooter version={packageAppVersion} />);
    expect(html).toContain("noxcode");
    expect(html).toContain(`v${packageAppVersion}`);
    expect(html).not.toContain(">v0.2<");
  });
});
