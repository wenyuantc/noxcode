import { describe, expect, it } from "vitest";

import { clampGitPanelWidth, gitPanelLayout } from "./gitPanelLayout";

describe("Git panel layout", () => {
  it("clamps preferences and replaces non-finite widths", () => {
    expect(clampGitPanelWidth(100)).toBe(320);
    expect(clampGitPanelWidth(900)).toBe(800);
    expect(clampGitPanelWidth(560)).toBe(560);
    expect(clampGitPanelWidth(Number.NaN)).toBe(380);
    expect(clampGitPanelWidth(Infinity)).toBe(380);
  });

  it("reserves 400 pixels for the session and includes the resize handle", () => {
    expect(gitPanelLayout(1000, 800)).toEqual({
      overlay: false,
      minWidth: 320,
      maxWidth: 596,
      width: 596,
    });
    expect(gitPanelLayout(724, 380).width).toBe(320);
    expect(gitPanelLayout(2000, 800).maxWidth).toBe(800);
  });

  it("uses an overlay when the two columns no longer fit", () => {
    expect(gitPanelLayout(723, 800)).toEqual({
      overlay: true,
      minWidth: 320,
      maxWidth: 719,
      width: 719,
    });
    expect(gitPanelLayout(300, 380)).toEqual({
      overlay: true,
      minWidth: 296,
      maxWidth: 296,
      width: 296,
    });
    expect(gitPanelLayout(0, 380).width).toBe(0);
    expect(gitPanelLayout(Number.NaN, 380).width).toBe(0);
  });

  it("does not lose the preferred width when the window temporarily shrinks", () => {
    expect(gitPanelLayout(900, 700).width).toBe(496);
    expect(gitPanelLayout(1500, 700).width).toBe(700);
  });
});
