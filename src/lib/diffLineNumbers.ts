export type DiffLineType = "meta" | "hunk" | "add" | "del" | "ctx" | "note";

export interface DiffLineInfo {
  type: DiffLineType;
  text: string;
  oldLine: number | null;
  newLine: number | null;
}

const HUNK_RE = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/;

function isMetaLine(text: string): boolean {
  return (
    text.startsWith("diff ") ||
    text.startsWith("index ") ||
    text.startsWith("---") ||
    text.startsWith("+++") ||
    text.startsWith("new file") ||
    text.startsWith("deleted file") ||
    text.startsWith("old mode") ||
    text.startsWith("new mode") ||
    text.startsWith("similarity ") ||
    text.startsWith("rename ") ||
    text.startsWith("copy ") ||
    text.startsWith("Binary ") ||
    text.startsWith("GIT binary")
  );
}

export function parseDiffLineNumbers(patch: string): DiffLineInfo[] {
  const rawLines = patch.split("\n");
  let oldLine = 0;
  let newLine = 0;

  return rawLines.map((text) => {
    if (text.startsWith("@@")) {
      const match = text.match(HUNK_RE);
      if (match) {
        oldLine = Number(match[1]);
        newLine = Number(match[2]);
      }
      return { type: "hunk", text, oldLine: null, newLine: null };
    }
    if (isMetaLine(text)) {
      return { type: "meta", text, oldLine: null, newLine: null };
    }
    if (text.startsWith("\\")) {
      return { type: "note", text, oldLine: null, newLine: null };
    }
    if (text.startsWith("+")) {
      const line = newLine;
      newLine += 1;
      return { type: "add", text, oldLine: null, newLine: line };
    }
    if (text.startsWith("-")) {
      const line = oldLine;
      oldLine += 1;
      return { type: "del", text, oldLine: line, newLine: null };
    }
    const currentOld = oldLine;
    const currentNew = newLine;
    oldLine += 1;
    newLine += 1;
    return { type: "ctx", text, oldLine: currentOld, newLine: currentNew };
  });
}

export function diffGutterWidth(lines: DiffLineInfo[]): number {
  let max = 0;
  for (const line of lines) {
    if (line.oldLine != null) max = Math.max(max, line.oldLine);
    if (line.newLine != null) max = Math.max(max, line.newLine);
  }
  return Math.max(2, String(max).length);
}
