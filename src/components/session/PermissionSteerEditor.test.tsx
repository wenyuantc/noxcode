import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PermissionSteerEditor } from "./PermissionSteerEditor";

const harness = vi.hoisted(() => ({
  cursor: 0,
  setters: [vi.fn(), vi.fn(), vi.fn(), vi.fn()],
  submit: vi.fn(),
  actions: new Map<string, () => unknown>(),
}));
vi.mock("react", async (original) => {
  const actual = await original<typeof import("react")>();
  return {
    ...actual,
    useState: () => {
      const index = harness.cursor++;
      const values = [
        "change direction",
        [{ name: "image.png", path: "/staged/image.png" }],
        false,
        null,
      ];
      return [values[index], harness.setters[index]];
    },
  };
});
vi.mock("@/hooks/useNativeSteer", () => ({
  useNativeSteer: () => ({
    snapshot: { turn_id: "turn" },
    busy: false,
    error: null,
    canSubmit: true,
    submit: harness.submit,
  }),
}));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, args?: { name: string }) => (args?.name ? `${key}: ${args.name}` : key),
  }),
}));
vi.mock("@/components/ui/button", () => ({
  Button: ({ children, onClick }: { children: ReactNode; onClick?: () => unknown }) => {
    if (onClick) harness.actions.set(String(children), onClick);
    return <button>{children}</button>;
  },
}));

describe("permission modal inline steering", () => {
  beforeEach(() => {
    harness.cursor = 0;
    harness.submit.mockReset();
    harness.actions.clear();
    harness.setters.forEach((setter) => setter.mockReset());
  });
  it("labels the owning session and cancellation returns to authorization without submitting", async () => {
    const onEditingChange = vi.fn();
    const html = renderToStaticMarkup(
      <PermissionSteerEditor
        sessionId="owner"
        label="Owning session"
        editing
        onEditingChange={onEditingChange}
      />,
    );
    expect(html).toContain("Owning session");
    expect(html).toContain('aria-label="steer.editor"');
    expect(html).toContain("image.png");
    await harness.actions.get("steer.back")!();
    expect(onEditingChange).toHaveBeenCalledWith(false);
    expect(harness.submit).not.toHaveBeenCalled();
    expect(harness.setters[0]).not.toHaveBeenCalled();
    expect(harness.setters[1]).not.toHaveBeenCalled();
  });
  it("uses the same text-plus-attachment submit path and retains draft on rejection", async () => {
    harness.submit.mockResolvedValue(false);
    const onEditingChange = vi.fn();
    renderToStaticMarkup(
      <PermissionSteerEditor
        sessionId="owner"
        label="Owning session"
        editing
        onEditingChange={onEditingChange}
      />,
    );
    await harness.actions.get("steer.action")!();
    expect(harness.submit).toHaveBeenCalledWith("change direction", ["/staged/image.png"]);
    expect(harness.setters[0]).not.toHaveBeenCalled();
    expect(harness.setters[1]).not.toHaveBeenCalled();
    expect(onEditingChange).not.toHaveBeenCalled();
  });
  it("successful steering leaves a surviving child permission at its existing actions", async () => {
    harness.submit.mockResolvedValue(true);
    const onEditingChange = vi.fn();
    renderToStaticMarkup(
      <PermissionSteerEditor
        sessionId="owner"
        label="Owning session"
        editing
        onEditingChange={onEditingChange}
      />,
    );
    await harness.actions.get("steer.action")!();
    expect(harness.setters[0]).toHaveBeenCalledWith("");
    expect(harness.setters[1]).toHaveBeenCalledWith([]);
    expect(onEditingChange).toHaveBeenCalledWith(false);
  });
});
