import { renderToStaticMarkup } from "react-dom/server";
import { I18nextProvider } from "react-i18next";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { ChannelTestButton } from "./ChannelTestButton";

function renderButton(props: React.ComponentProps<typeof ChannelTestButton>) {
  return renderToStaticMarkup(
    <I18nextProvider i18n={i18n}>
      <ChannelTestButton {...props} />
    </I18nextProvider>,
  );
}

describe("ChannelTestButton", () => {
  it("renders single button when there are no models", () => {
    const html = renderButton({
      models: [],
      onTest: () => undefined,
    });

    expect(html).toContain("测通");
    expect(html).not.toContain("选择模型测通");
    expect(html).not.toContain('data-slot="dropdown-menu-trigger"');
  });

  it("renders single button when there is only one model", () => {
    const html = renderButton({
      models: [{ id: "gpt-4o" }],
      defaultModelId: "gpt-4o",
      onTest: () => undefined,
    });

    expect(html).toContain("测通");
    expect(html).not.toContain("选择模型测通");
  });

  it("renders split button with dropdown trigger when there are multiple models", () => {
    const html = renderButton({
      models: [{ id: "gpt-4o" }, { id: "gpt-4o-mini" }, { id: "o3-mini" }],
      defaultModelId: "gpt-4o-mini",
      onTest: () => undefined,
    });

    expect(html).toContain("测通");
    expect(html).toContain("选择模型测通");
    expect(html).toContain('title="测通 (gpt-4o-mini)"');
  });

  it("falls back to the first model when defaultModelId is not specified or invalid", () => {
    const html = renderButton({
      models: [{ id: "claude-3-5-sonnet" }, { id: "claude-3-haiku" }],
      defaultModelId: null,
      onTest: () => undefined,
    });

    expect(html).toContain('title="测通 (claude-3-5-sonnet)"');
  });

  it("displays loading spinner and disabled state when isTesting is true", () => {
    const html = renderButton({
      models: [{ id: "gpt-4o" }, { id: "gpt-4o-mini" }],
      isTesting: true,
      onTest: () => undefined,
    });

    expect(html).toContain("animate-spin");
    expect(html).toContain("disabled");
  });
});
