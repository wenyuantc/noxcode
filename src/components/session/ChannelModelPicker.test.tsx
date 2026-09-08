import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { useChannelStore } from "@/stores/channelStore";
import { ChannelModelPicker } from "./ChannelModelPicker";

function renderPicker(node: ReactNode) {
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <MemoryRouter>{node}</MemoryRouter>
    </I18nextProvider>,
  );
}

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

  it("renders a controlled selection without pending or persist side effects", () => {
    useChannelStore.setState({
      channels: [
        {
          id: "c1",
          name: "Myai-ollama",
          protocol: "openai",
          base_url: "http://localhost",
          extra_headers_json: null,
          models: [
            {
              id: "deepseek-v4-flash",
              context_tokens: null,
              max_output_tokens: null,
              thinking_enabled: null,
              thinking_level: null,
              thinking_levels: null,
              input_types: null,
            },
          ],
          responses_continuation: "auto",
          enabled: true,
          api_key: null,
          api_key_configured: false,
          created_at: "",
          updated_at: "",
        },
      ],
    });
    const html = renderPicker(
      <ChannelModelPicker
        selection={{ channelId: "c1", modelId: "deepseek-v4-flash" }}
        onSelectionChange={() => {
          throw new Error("controlled picker must not persist");
        }}
        className="max-w-[min(16rem,40vw)]"
      />,
    );
    expect(html).toContain("c1/deepseek-v4-flash");
    expect(html).toContain("max-w-[min(16rem,40vw)]");
    expect(html).not.toContain("text-amber-500");
  });
});
