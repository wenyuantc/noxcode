import type { GitStatus, GitStatusEntry } from "@/lib/types";

function isXyChanged(char: string | undefined): boolean {
  return char !== undefined && char !== " " && char !== "." && char !== "?" && char !== "!";
}

export function isStaged(entry: GitStatusEntry): boolean {
  const xy = entry.xy.padEnd(2, " ");
  return isXyChanged(xy[0]);
}

export function isUnstaged(entry: GitStatusEntry): boolean {
  const xy = entry.xy.padEnd(2, " ");
  return isXyChanged(xy[1]);
}

export function isUntracked(entry: GitStatusEntry): boolean {
  return entry.kind === "untracked" || entry.xy.startsWith("?");
}

export function groupGitStatus(status: GitStatus | null) {
  const entries = status?.entries ?? [];
  return {
    staged: entries.filter((entry) => isStaged(entry) && !isUntracked(entry)),
    unstaged: entries.filter((entry) => isUnstaged(entry) && !isUntracked(entry)),
    untracked: entries.filter(isUntracked),
  };
}

export function diffLineClass(line: string): string {
  if (line.startsWith("+++") || line.startsWith("---") || line.startsWith("diff ")) {
    return "text-muted-foreground";
  }
  if (line.startsWith("+")) return "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300";
  if (line.startsWith("-")) return "bg-rose-500/10 text-rose-700 dark:text-rose-300";
  if (line.startsWith("@@")) return "text-sky-700 dark:text-sky-300";
  return "text-foreground";
}
