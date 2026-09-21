import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";
import { SteerInputs } from "./SteerInputs";

const restore = vi.hoisted(() => vi.fn());
const actions = vi.hoisted(() => new Map<string, () => unknown>());
vi.mock("@/stores/steerStore", () => ({
  useSteerStore: (selector: (state: unknown) => unknown) =>
    selector({
      snapshots: {
        s: {
          receipts: [
            {
              input_id: "i",
              text: "recovered text",
              status: "cancelled",
              error: "interrupted",
              image_count: 2,
            },
          ],
        },
      },
    }),
}));
vi.mock("@/stores/uiStore", () => ({
  useUiStore: (selector: (state: unknown) => unknown) =>
    selector({ composerDraft: "existing", setComposerDraft: restore }),
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("@/components/ui/button", () => ({
  Button: ({ children, onClick }: { children: ReactNode; onClick?: () => unknown }) => {
    if (onClick) actions.set(String(children), onClick);
    return <button>{children}</button>;
  },
}));

describe("interrupted steer recovery", () => {
  it("shows interruption and image reselection, restoring text without overwriting another draft", async () => {
    const html = renderToStaticMarkup(<SteerInputs sessionId="s" />);
    expect(html).toContain("steer.cancelled");
    expect(html).toContain("recovered text");
    expect(html).toContain("steer.reselect");
    await actions.get("steer.restore")!();
    expect(restore).toHaveBeenCalledWith("existing\nrecovered text");
  });
});
