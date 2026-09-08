import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { ChannelModelPicker } from "./ChannelModelPicker";

describe("ChannelModelPicker", () => {
  it("renders trigger with auto width (max-w-full)", () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter>
          <ChannelModelPicker />
        </MemoryRouter>
      </I18nextProvider>,
    );

    expect(html).toContain("max-w-full");
    expect(html).not.toContain("max-w-[min(12rem,100%)]");
    expect(html).not.toContain("max-w-48");
    expect(html).not.toContain('disabled=""');
  });
});
