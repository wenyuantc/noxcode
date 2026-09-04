import { describe, expect, it } from "vitest";

import { languageFromClassName, languageFromPath, normalizeLanguage } from "./codeLanguage";

describe("codeLanguage", () => {
  it("normalizes aliases and empty values", () => {
    expect(normalizeLanguage(undefined)).toBe("plaintext");
    expect(normalizeLanguage("TS")).toBe("typescript");
    expect(normalizeLanguage("js")).toBe("javascript");
    expect(normalizeLanguage("yml")).toBe("yaml");
    expect(normalizeLanguage("text")).toBe("plaintext");
  });

  it("reads a language class from markdown fences", () => {
    expect(languageFromClassName("language-tsx")).toBe("tsx");
    expect(languageFromClassName("language-python extra")).toBe("python");
    expect(languageFromClassName("not-a-language")).toBe("plaintext");
  });

  it("infers a language from file paths", () => {
    expect(languageFromPath("src/app/main.rs")).toBe("rust");
    expect(languageFromPath("components/Widget.tsx")).toBe("tsx");
    expect(languageFromPath("Dockerfile")).toBe("dockerfile");
    expect(languageFromPath("Makefile")).toBe("makefile");
    expect(languageFromPath("notes")).toBe("plaintext");
    expect(languageFromPath("C:\\\\repo\\\\pkg\\\\index.ts")).toBe("typescript");
  });
});
