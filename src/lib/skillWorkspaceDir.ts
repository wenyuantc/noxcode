import type { NativeSkill, Workspace } from "@/lib/types";

export const SKILL_DIR_ALL = "all";
export const SKILL_DIR_GLOBAL = "global";
export const SKILL_DIR_PLUGIN = "plugin";
export const SKILL_DIR_WORKSPACE_PREFIX = "ws:";

const PROJECT_MARKERS = [
  "/.noxcode/skills",
  "/.zcode/skills",
  "/.agents/skills",
  "/.claude/skills",
];

export function normalizeSkillPath(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+$/, "");
}

export function skillWorkspaceDir(skill: Pick<NativeSkill, "source" | "dir">): string {
  if (skill.source === "global") return SKILL_DIR_GLOBAL;
  if (skill.source === "plugin") return SKILL_DIR_PLUGIN;
  const path = normalizeSkillPath(skill.dir);
  const lower = path.toLowerCase();
  for (const marker of PROJECT_MARKERS) {
    const index = lower.lastIndexOf(marker);
    if (index >= 0) return path.slice(0, index);
  }
  return path;
}

export function workspaceSkillDirFilter(workspaceId: string): string {
  return `${SKILL_DIR_WORKSPACE_PREFIX}${workspaceId}`;
}

export function parseWorkspaceSkillDirFilter(dir: string): string | null {
  if (!dir.startsWith(SKILL_DIR_WORKSPACE_PREFIX)) return null;
  const id = dir.slice(SKILL_DIR_WORKSPACE_PREFIX.length);
  return id || null;
}

export function skillBelongsToDir(
  skill: Pick<NativeSkill, "source" | "dir">,
  dir: string,
): boolean {
  if (dir === SKILL_DIR_ALL) return true;
  if (dir === SKILL_DIR_GLOBAL) return skill.source === "global";
  if (dir === SKILL_DIR_PLUGIN) return skill.source === "plugin";
  if (parseWorkspaceSkillDirFilter(dir)) {
    return skill.source.startsWith("workspace_");
  }
  const root = normalizeSkillPath(dir);
  const project = skillWorkspaceDir(skill);
  if (project === SKILL_DIR_GLOBAL || project === SKILL_DIR_PLUGIN) return false;
  return project === root || project.startsWith(`${root}/`);
}

export function uniqueSkillWorkspaceDirs(
  skills: Array<Pick<NativeSkill, "source" | "dir">>,
): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const skill of skills) {
    const dir = skillWorkspaceDir(skill);
    if (dir === SKILL_DIR_GLOBAL || dir === SKILL_DIR_PLUGIN || seen.has(dir)) continue;
    seen.add(dir);
    out.push(dir);
  }
  return out.sort((left, right) => left.localeCompare(right));
}

export function shortWorkspaceDirLabel(dir: string): string {
  const path = normalizeSkillPath(dir);
  const parts = path.split("/").filter(Boolean);
  if (parts.length <= 2) return path;
  return parts.slice(-2).join("/");
}

export function workspaceSkillPath(
  workspace: Pick<Workspace, "workspace_type" | "repo_path" | "remote_repo_path">,
): string {
  const raw = workspace.workspace_type === "ssh" ? workspace.remote_repo_path : workspace.repo_path;
  return raw?.trim() ? normalizeSkillPath(raw) : "";
}
