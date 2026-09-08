import { describe, expect, it } from "vitest";

import { formatScaledTokens } from "./utils";

describe("formatScaledTokens", () => {
  it("keeps values under 1000 as a plain count", () => {
    expect(formatScaledTokens(0)).toBe("0");
    expect(formatScaledTokens(999)).toBe("999");
  });

  it("uses K after 1000 tokens and keeps two decimals", () => {
    expect(formatScaledTokens(1000)).toBe("1.00K");
    expect(formatScaledTokens(1500)).toBe("1.50K");
    expect(formatScaledTokens(39_000)).toBe("39.00K");
    expect(formatScaledTokens(12_340)).toBe("12.34K");
  });

  it("uses M after 1000K and B after 1000M, keeping two decimals", () => {
    expect(formatScaledTokens(1_000_000)).toBe("1.00M");
    expect(formatScaledTokens(1_560_000)).toBe("1.56M");
    expect(formatScaledTokens(1_000_000_000)).toBe("1.00B");
    expect(formatScaledTokens(2_300_000_000)).toBe("2.30B");
  });

  it("promotes 1000.00K to M", () => {
    expect(formatScaledTokens(999_500)).toBe("999.50K");
    expect(formatScaledTokens(999_995)).toBe("1.00M");
  });
});
