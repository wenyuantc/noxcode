import { I18nextProvider } from "react-i18next";
import { renderToString } from "react-dom/server";
import { describe, expect, it } from "vitest";

import i18n from "@/lib/i18n";
import { emptyChannelModel } from "@/lib/modelCatalog";
import type { AiChannelModel } from "@/lib/types";
import { ChannelModelsEditor } from "./ChannelModelsEditor";

function model(partial: Partial<AiChannelModel> & { id: string }): AiChannelModel {
  return {
    ...emptyChannelModel(partial.id),
    ...partial,
  };
}

function renderEditor(models: AiChannelModel[]) {
  return renderToString(
    <I18nextProvider i18n={i18n}>
      <ChannelModelsEditor
        models={models}
        catalog={[]}
        disabled={false}
        onChange={() => undefined}
      />
    </I18nextProvider>,
  );
}

describe("ChannelModelsEditor thinking levels", () => {
  it("hides thinking level checkboxes when thinking is turned off", () => {
    const html = renderEditor([
      model({
        id: "composer-2.5",
        thinking_enabled: false,
        thinking_levels: ["low", "medium", "high"],
      }),
    ]);
    expect(html).toContain("关闭思考");
    expect(html).not.toContain("channel-model-0-thinking-low");
    expect(html).not.toContain("允许的思考等级");
  });

  it("shows thinking level checkboxes when thinking is on", () => {
    const html = renderEditor([
      model({
        id: "composer-2.5",
        thinking_enabled: true,
        thinking_level: "medium",
        thinking_levels: ["low", "medium", "high"],
      }),
    ]);
    expect(html).toContain("开启思考");
    expect(html).toContain("允许的思考等级");
    expect(html).toContain("channel-model-0-thinking-low");
    expect(html).toContain("channel-model-0-thinking-medium");
  });
});
