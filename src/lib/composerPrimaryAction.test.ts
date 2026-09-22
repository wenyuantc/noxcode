import { describe, expect, it } from "vitest";

import { composerHasSubmittable, composerPrimaryAction } from "./composerPrimaryAction";

describe("composerHasSubmittable", () => {
  it("treats an empty draft as nothing to send", () => {
    expect(composerHasSubmittable({ draft: "", attachmentCount: 0, hasPills: false })).toBe(false);
  });

  it("treats whitespace as nothing to send", () => {
    expect(composerHasSubmittable({ draft: " \n\t ", attachmentCount: 0, hasPills: false })).toBe(
      false,
    );
  });

  it("accepts text, an attachment, or a pill on its own", () => {
    expect(composerHasSubmittable({ draft: "继续", attachmentCount: 0, hasPills: false })).toBe(
      true,
    );
    expect(composerHasSubmittable({ draft: "", attachmentCount: 1, hasPills: false })).toBe(true);
    expect(composerHasSubmittable({ draft: "  ", attachmentCount: 0, hasPills: true })).toBe(true);
  });
});

describe("composerPrimaryAction", () => {
  it("stays send while idle, whether or not there is content", () => {
    expect(composerPrimaryAction({ working: false, hasSubmittable: false, sendBusy: false })).toBe(
      "send",
    );
    expect(composerPrimaryAction({ working: false, hasSubmittable: true, sendBusy: false })).toBe(
      "send",
    );
  });

  it("shows stop while running with nothing to send", () => {
    expect(composerPrimaryAction({ working: true, hasSubmittable: false, sendBusy: false })).toBe(
      "stop",
    );
  });

  it("switches to send while running once there is content", () => {
    expect(composerPrimaryAction({ working: true, hasSubmittable: true, sendBusy: false })).toBe(
      "send",
    );
  });

  it("keeps send while a send is in flight, even if the draft is already empty", () => {
    expect(composerPrimaryAction({ working: true, hasSubmittable: false, sendBusy: true })).toBe(
      "send",
    );
  });

  it("returns to stop after the draft is cleared and the run is still going", () => {
    const cleared = composerHasSubmittable({ draft: "", attachmentCount: 0, hasPills: false });
    expect(composerPrimaryAction({ working: true, hasSubmittable: cleared, sendBusy: false })).toBe(
      "stop",
    );
  });
});
