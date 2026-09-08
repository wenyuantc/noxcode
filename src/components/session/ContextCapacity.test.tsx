import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { ContextCapacity } from "./ContextCapacity";

describe("ContextCapacity", () => {
  it("shows scaled total usage next to the cache rate", () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <ContextCapacity
          open
          usage={{
            session_record_id: "s1",
            used_tokens: 39_000,
            limit_tokens: 1_000_000,
            generation: 0,
            compactions: 0,
            prompt_tokens: 200,
            cached_tokens: 0,
          }}
        />
      </I18nextProvider>,
    );

    expect(html).toContain("缓存率 0.0%");
    expect(html).toContain("总用量 39.00K");
  });

  it("uses billed session tokens including subagent usage instead of context occupancy", () => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <ContextCapacity
          open
          totalTokens={66_768}
          usage={{
            session_record_id: "s1",
            used_tokens: 39_000,
            limit_tokens: 1_000_000,
            generation: 0,
            compactions: 0,
            prompt_tokens: 200,
            cached_tokens: 0,
          }}
        />
      </I18nextProvider>,
    );

    expect(html).toContain("总用量 66.77K");
    expect(html).not.toContain("总用量 39.00K");
  });
});
