import { describe, expect, it } from "vitest";

import { isNearBottom, pinAfterUserScroll } from "./sessionScroll";

describe("isNearBottom", () => {
  it("is true at the bottom", () => {
    expect(isNearBottom({ scrollHeight: 1000, scrollTop: 600, clientHeight: 400 })).toBe(true);
  });

  it("is true within the 80px threshold", () => {
    expect(isNearBottom({ scrollHeight: 1000, scrollTop: 520, clientHeight: 400 })).toBe(true);
  });

  it("is false beyond the threshold", () => {
    expect(isNearBottom({ scrollHeight: 1000, scrollTop: 519, clientHeight: 400 })).toBe(false);
  });

  it("does not treat an unlaid-out container as away from bottom", () => {
    expect(isNearBottom({ scrollHeight: 0, scrollTop: 0, clientHeight: 0 })).toBe(true);
    expect(isNearBottom({ scrollHeight: 2000, scrollTop: 0, clientHeight: 0 })).toBe(true);
  });
});

describe("pinAfterUserScroll", () => {
  it("keeps the previous pin during programmatic scroll", () => {
    expect(
      pinAfterUserScroll({
        programmatic: true,
        clientHeight: 400,
        nearBottom: false,
        previous: true,
      }),
    ).toBe(true);
    expect(
      pinAfterUserScroll({
        programmatic: true,
        clientHeight: 400,
        nearBottom: true,
        previous: false,
      }),
    ).toBe(false);
  });

  it("keeps the previous pin when the container is not laid out", () => {
    expect(
      pinAfterUserScroll({
        programmatic: false,
        clientHeight: 0,
        nearBottom: true,
        previous: false,
      }),
    ).toBe(false);
  });

  it("unpins when the user scrolls away", () => {
    expect(
      pinAfterUserScroll({
        programmatic: false,
        clientHeight: 400,
        nearBottom: false,
        previous: true,
      }),
    ).toBe(false);
  });

  it("pins when the user scrolls back to the bottom", () => {
    expect(
      pinAfterUserScroll({
        programmatic: false,
        clientHeight: 400,
        nearBottom: true,
        previous: false,
      }),
    ).toBe(true);
  });
});
