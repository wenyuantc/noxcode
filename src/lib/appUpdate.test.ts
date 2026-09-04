import { describe, expect, it } from "vitest";

import { mapUpdaterError, updaterErrorI18nKey } from "@/lib/appUpdate";

describe("mapUpdaterError", () => {
  it("maps a missing update to already latest", () => {
    expect(mapUpdaterError(null)).toBe("already_latest");
    expect(mapUpdaterError(undefined)).toBe("already_latest");
    expect(mapUpdaterError("")).toBe("already_latest");
    expect(mapUpdaterError("Already up to date")).toBe("already_latest");
    expect(mapUpdaterError("no updates available")).toBe("already_latest");
  });

  it("maps network failures", () => {
    expect(mapUpdaterError("Could not fetch a valid release JSON from the remote")).toBe("network");
    expect(mapUpdaterError(new Error("error sending request for url (https://example)"))).toBe(
      "network",
    );
    expect(mapUpdaterError("dns error: failed to lookup address")).toBe("network");
    expect(mapUpdaterError("connection timed out")).toBe("network");
  });

  it("maps missing or invalid signatures", () => {
    expect(
      mapUpdaterError(
        "The signature abc could not be decoded, please check if it is a valid base64 string.",
      ),
    ).toBe("signature");
    expect(mapUpdaterError("minisign: invalid signature")).toBe("signature");
    expect(mapUpdaterError("failed to verify public key")).toBe("signature");
  });

  it("maps development mode refusals", () => {
    expect(mapUpdaterError("Updates are not available in development mode")).toBe("dev_mode");
    expect(mapUpdaterError("Unable to check for an update on macOS in development mode")).toBe(
      "dev_mode",
    );
    expect(mapUpdaterError("tauri dev cannot install updates")).toBe("dev_mode");
  });

  it("maps user cancellation", () => {
    expect(mapUpdaterError("Authentication failed or was cancelled")).toBe("cancelled");
    expect(mapUpdaterError("Update canceled by user")).toBe("cancelled");
  });

  it("falls back to unknown for other failures", () => {
    expect(mapUpdaterError("the platform `windows-arm64` was not found")).toBe("unknown");
    expect(mapUpdaterError({ message: 12 })).toBe("unknown");
  });
});

describe("updaterErrorI18nKey", () => {
  it("maps codes to settings about error keys", () => {
    expect(updaterErrorI18nKey("already_latest")).toBe("about.errors.alreadyLatest");
    expect(updaterErrorI18nKey("dev_mode")).toBe("about.errors.devMode");
    expect(updaterErrorI18nKey("network")).toBe("about.errors.network");
    expect(updaterErrorI18nKey("signature")).toBe("about.errors.signature");
    expect(updaterErrorI18nKey("cancelled")).toBe("about.errors.cancelled");
    expect(updaterErrorI18nKey("unknown")).toBe("about.errors.unknown");
  });
});
