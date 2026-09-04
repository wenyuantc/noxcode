import { describe, expect, it } from "vitest";

import {
  parseWorkspaceSkillDirFilter,
  shortWorkspaceDirLabel,
  skillBelongsToDir,
  skillWorkspaceDir,
  uniqueSkillWorkspaceDirs,
  workspaceSkillDirFilter,
  workspaceSkillPath,
} from "./skillWorkspaceDir";

describe("skillWorkspaceDir", () => {
  it("maps global and plugin sources", () => {
    expect(skillWorkspaceDir({ source: "global", dir: "/Users/me/.noxcode/skills/demo" })).toBe(
      "global",
    );
    expect(skillWorkspaceDir({ source: "plugin", dir: "/plugins/foo/skills/demo" })).toBe("plugin");
  });

  it("strips project skill roots", () => {
    expect(
      skillWorkspaceDir({
        source: "workspace_zcode",
        dir: "/Users/me/proj/.zcode/skills/review",
      }),
    ).toBe("/Users/me/proj");
    expect(
      skillWorkspaceDir({
        source: "workspace_noxcode",
        dir: "/Users/me/proj/apps/web/.noxcode/skills/local",
      }),
    ).toBe("/Users/me/proj/apps/web");
  });
});

describe("skillBelongsToDir", () => {
  const skill = {
    source: "workspace_zcode" as const,
    dir: "/Users/me/proj/.zcode/skills/review",
  };

  it("matches the project root and ancestors", () => {
    expect(skillBelongsToDir(skill, "all")).toBe(true);
    expect(skillBelongsToDir(skill, "/Users/me/proj")).toBe(true);
    expect(skillBelongsToDir(skill, "/Users/me")).toBe(true);
    expect(skillBelongsToDir(skill, "/Users/other")).toBe(false);
    expect(skillBelongsToDir(skill, "global")).toBe(false);
  });

  it("treats workspace id filters as project-only", () => {
    expect(skillBelongsToDir(skill, workspaceSkillDirFilter("ws-1"))).toBe(true);
    expect(
      skillBelongsToDir({ source: "global", dir: "/Users/me/.noxcode/skills/demo" }, "ws:ws-1"),
    ).toBe(false);
    expect(parseWorkspaceSkillDirFilter("ws:abc")).toBe("abc");
    expect(parseWorkspaceSkillDirFilter("ws:")).toBeNull();
  });
});

describe("uniqueSkillWorkspaceDirs", () => {
  it("lists project dirs and skips global/plugin", () => {
    expect(
      uniqueSkillWorkspaceDirs([
        { source: "global", dir: "/Users/me/.noxcode/skills/a" },
        { source: "workspace_zcode", dir: "/Users/me/proj/.zcode/skills/a" },
        { source: "workspace_agents", dir: "/Users/me/proj/.agents/skills/b" },
        { source: "plugin", dir: "/p/skills/c" },
      ]),
    ).toEqual(["/Users/me/proj"]);
  });
});

describe("workspace labels", () => {
  it("shortens long paths", () => {
    expect(shortWorkspaceDirLabel("/Users/me/IdeaProjects/hub")).toBe("IdeaProjects/hub");
  });

  it("reads local and ssh workspace paths", () => {
    expect(
      workspaceSkillPath({
        workspace_type: "local",
        repo_path: "/Users/me/proj/",
        remote_repo_path: null,
      }),
    ).toBe("/Users/me/proj");
    expect(
      workspaceSkillPath({
        workspace_type: "ssh",
        repo_path: null,
        remote_repo_path: "/home/me/app",
      }),
    ).toBe("/home/me/app");
  });
});
