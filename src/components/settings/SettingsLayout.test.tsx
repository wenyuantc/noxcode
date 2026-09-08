import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { packageAppVersion } from "@/lib/appUpdate";

import { SettingsBrandFooter } from "./SettingsLayout";

describe("SettingsBrandFooter", () => {
  it("renders the current package version instead of a stale hardcoded label", () => {
    const html = renderToString(<SettingsBrandFooter version={packageAppVersion} />);
    expect(html).toContain("noxcode");
    expect(html).toContain(`v${packageAppVersion}`);
    expect(html).not.toContain(">v0.2<");
  });
});
