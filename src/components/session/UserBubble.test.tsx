import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

import { UserBubble } from "./UserBubble";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock("@/lib/backend", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/backend")>();
  return {
    ...actual,
    applyNativeFileRollback: vi.fn(),
    applyNativeHistoryBoundary: vi.fn(),
    previewNativeFileRollback: vi.fn(),
  };
});

describe("UserBubble", () => {
  it("renders user bubble with break-words and anywhere overflow wrapping to prevent layout blowout", () => {
    const longUrl =
      "data:text/html,%3Cinput%20id%3Dn%3E%3Cbutton%20onclick%3D%22document.body.append(document.getElementById('n').value)%22%3EGo%3C%2Fbutton%3E";
    const prompt = `验收：用 Playwright 打开 ${longUrl} 并确认结果`;

    const html = renderToStaticMarkup(
      <UserBubble text={prompt} sessionId="session-1" editable={true} working={false} />,
    );

    expect(html).toContain("break-words");
    expect(html).toContain("[overflow-wrap:anywhere]");
    expect(html).toContain("min-w-0");
    expect(html).toContain("max-w-full");
    expect(html).toContain("%3Cbutton");
  });
});
