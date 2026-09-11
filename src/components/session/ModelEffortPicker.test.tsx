import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { MemoryRouter } from "react-router-dom";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { useChannelStore } from "@/stores/channelStore";
import { useSessionStore } from "@/stores/sessionStore";
import { ModelEffortPicker } from "./ModelEffortPicker";

describe("ModelEffortPicker", () => {
  it("renders trigger with model and effort label", () => {
    useSessionStore.setState({ selectedSessionId: null });
    useChannelStore.setState({
      channels: [
        {
          id: "chan-1",
          name: "6 Astra",
          protocol: "openai",
          base_url: "http://localhost",
          extra_headers_json: null,
          models: [
            {
              id: "astra-v1",
              context_tokens: null,
              max_output_tokens: null,
              thinking_enabled: true,
              thinking_level: "high",
              thinking_levels: ["low", "high", "max"],
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
      activeChannelId: "chan-1",
      activeModelId: "astra-v1",
    });

    const html = renderToStaticMarkup(
      <I18nextProvider i18n={i18n}>
        <MemoryRouter>
          <ModelEffortPicker
            selection={{ channelId: "chan-1", modelId: "astra-v1" }}
            reasoningEffort="high"
          />
        </MemoryRouter>
      </I18nextProvider>,
    );

    expect(html).toContain("6 Astra/astra-v1");
    expect(html).toContain("max-w-64");
  });
});
