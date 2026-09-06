import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { ThinkingRow } from "./ThinkingRow";

describe("ThinkingRow", () => {
  it.each([
    ["saved", "[思考] 8秒\nThought details", "思考 · 持续了 8 秒"],
    ["streaming", "Thought details", "思考 · 持续了几秒"],
  ])("keeps %s thinking collapsed by default", (_kind, text, label) => {
    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <ThinkingRow
          items={[
            {
              id: "thinking-1",
              kind: "system",
              text,
              createdAt: "2026-01-01T00:00:00Z",
            },
          ]}
          nowMs={Date.parse("2026-01-01T00:00:00Z")}
        />
      </I18nextProvider>,
    );

    expect(html).toContain(label);
    expect(html).not.toContain("Thought details");
    expect(html).not.toContain("<pre");
    expect(html).toContain('aria-expanded="false"');
  });
});
