import { describe, expect, it } from "vitest";
import {
  fileAccessSelections,
  permissionDirectory,
  permissionTargetLabel,
} from "./nativeFileAccess";
import type { FileAccessPrompt } from "./types";

describe("file access selections", () => {
  const access: FileAccessPrompt = {
    target: { kind: "local" },
    paths: [
      {
        path: "/external/file.txt",
        requested_path: "../file.txt",
        capability: "edit",
        scope: "exact",
        operation: "write",
        outside_workspace: true,
      },
      {
        path: "/external/logs",
        requested_path: "/external/logs",
        capability: "read",
        scope: "subtree",
        operation: "search",
        outside_workspace: true,
      },
    ],
  };
  it("defaults to the target file and the searched directory", () => {
    expect(fileAccessSelections(access, {})).toEqual([
      { path: "/external/file.txt", directory: false },
      { path: "/external/logs", directory: true },
    ]);
  });
  it("sends the original approved target when choosing its directory", () => {
    expect(fileAccessSelections(access, { 0: true })[0]).toEqual({
      path: "/external/file.txt",
      directory: true,
    });
  });
  it("displays parent directories without truncating roots", () => {
    expect(permissionDirectory("/file.txt")).toBe("/");
    expect(permissionDirectory("/external/file.txt")).toBe("/external");
    expect(permissionDirectory("C:\\file.txt")).toBe("C:\\");
    expect(permissionDirectory("C:\\external\\file.txt")).toBe("C:\\external");
  });
  it("identifies the remote host and account", () => {
    expect(
      permissionTargetLabel({
        kind: "ssh",
        config_id: "one",
        host: "server",
        port: 2222,
        username: "user",
      }),
    ).toBe("user@server:2222");
  });
});
