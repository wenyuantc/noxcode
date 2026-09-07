import type { FileAccessPrompt, FileAccessSelection, PermissionTarget } from "./types";

export function permissionTargetLabel(target: PermissionTarget): string {
  return target.kind === "local" ? "" : `${target.username}@${target.host}:${target.port}`;
}

export function fileAccessSelections(
  access: FileAccessPrompt,
  directories: Record<number, boolean>,
): FileAccessSelection[] {
  return access.paths.map((entry, index) => ({
    path: entry.path,
    directory: directories[index] ?? entry.scope === "subtree",
  }));
}

export function permissionDirectory(path: string): string {
  const separator = /^[A-Za-z]:\\|^\\\\/.test(path) ? "\\" : "/";
  const last = path.lastIndexOf(separator);
  if (last <= 0) return separator;
  if (last === 2 && separator === "\\") return path.slice(0, 3);
  return path.slice(0, last);
}
