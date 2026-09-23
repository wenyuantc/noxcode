export const MAX_COMPOSER_IMAGES = 8;
export const MAX_COMPOSER_IMAGE_BYTES = 8 * 1024 * 1024;

export const COMPOSER_IMAGE_MIMES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;

export const COMPOSER_IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp"] as const;

export const COMPOSER_TEXT_EXTENSIONS = [
  "html",
  "htm",
  "txt",
  "md",
  "markdown",
  "json",
  "csv",
  "xml",
  "css",
  "js",
  "jsx",
  "mjs",
  "cjs",
  "ts",
  "tsx",
  "py",
  "rs",
  "go",
  "java",
  "kt",
  "c",
  "h",
  "cc",
  "cpp",
  "hpp",
  "cs",
  "rb",
  "php",
  "sh",
  "bash",
  "zsh",
  "yml",
  "yaml",
  "toml",
  "sql",
  "log",
  "vue",
  "svelte",
] as const;

export const COMPOSER_OFFICE_EXTENSIONS = ["xls", "xlsx", "doc", "docx"] as const;

export const COMPOSER_ATTACHMENT_EXTENSIONS = [
  ...COMPOSER_IMAGE_EXTENSIONS,
  "pdf",
  "mp4",
  ...COMPOSER_TEXT_EXTENSIONS,
  ...COMPOSER_OFFICE_EXTENSIONS,
] as const;

export const COMPOSER_FILE_ACCEPT = [
  "image/png",
  "image/jpeg",
  "image/gif",
  "image/webp",
  "application/pdf",
  "video/mp4",
  ...COMPOSER_ATTACHMENT_EXTENSIONS.map((extension) => `.${extension}`),
].join(",");

export const COMPOSER_DIALOG_FILTERS = [
  { name: "附件", extensions: [...COMPOSER_ATTACHMENT_EXTENSIONS] },
];

const TEXT_EXTENSIONS = new Set<string>(COMPOSER_TEXT_EXTENSIONS);

export type ComposerImageSkipReason = "mime" | "size" | "limit";

export type ComposerTriggerChar = "@" | "/" | "$";

export interface ComposerImageFileLike {
  name: string;
  type: string;
  size: number;
}

export interface ComposerImageItem {
  id: string;
  name: string;
  path: string;
  previewUrl: string;
  selected: boolean;
}

export interface ComposerImageSkip {
  name: string;
  reason: ComposerImageSkipReason;
}

export interface FilterComposerImagesResult<T> {
  accepted: T[];
  skipped: ComposerImageSkip[];
}

export interface MergeComposerImagesResult<T> {
  items: T[];
  skipped: ComposerImageSkip[];
}

export function composerImageExtension(name: string): string | null {
  const trimmed = name.trim();
  const dot = trimmed.lastIndexOf(".");
  if (dot < 0 || dot === trimmed.length - 1) return null;
  return trimmed.slice(dot + 1).toLowerCase();
}

export function composerImageMimeFromName(name: string): string | null {
  const extension = composerImageExtension(name);
  switch (extension) {
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "gif":
      return "image/gif";
    case "webp":
      return "image/webp";
    case "pdf":
      return "application/pdf";
    case "mp4":
      return "video/mp4";
    case "xls":
      return "application/vnd.ms-excel";
    case "xlsx":
      return "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
    case "doc":
      return "application/msword";
    case "docx":
      return "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
    default:
      if (!extension || !TEXT_EXTENSIONS.has(extension)) return null;
      if (extension === "html" || extension === "htm") return "text/html";
      if (extension === "json") return "application/json";
      if (extension === "xml") return "application/xml";
      return "text/plain";
  }
}

export function composerMediaByteLimit(name: string, mime = ""): number {
  const kind = mime.trim().toLowerCase() || composerImageMimeFromName(name) || "";
  if (
    kind === "application/pdf" ||
    kind === "application/vnd.ms-excel" ||
    kind === "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" ||
    kind === "application/msword" ||
    kind === "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
  ) {
    return 32 * 1024 * 1024;
  }
  if (kind === "video/mp4") return 6 * 1024 * 1024;
  if (kind.startsWith("text/") || kind === "application/json" || kind === "application/xml") {
    return 1024 * 1024;
  }
  return MAX_COMPOSER_IMAGE_BYTES;
}

export function isComposerImageMime(mime: string): boolean {
  return (COMPOSER_IMAGE_MIMES as readonly string[]).includes(mime.trim().toLowerCase());
}

export function isComposerImageFile(file: ComposerImageFileLike): boolean {
  const mime = file.type.trim().toLowerCase();
  if (mime && isComposerImageMime(mime)) return true;
  return composerImageMimeFromName(file.name) !== null;
}

export function filterComposerImageFiles<T extends ComposerImageFileLike>(
  files: readonly T[],
): FilterComposerImagesResult<T> {
  const accepted: T[] = [];
  const skipped: ComposerImageSkip[] = [];
  for (const file of files) {
    if (!isComposerImageFile(file)) {
      skipped.push({ name: file.name || "image", reason: "mime" });
      continue;
    }
    if (file.size > composerMediaByteLimit(file.name, file.type)) {
      skipped.push({ name: file.name || "image", reason: "size" });
      continue;
    }
    accepted.push(file);
  }
  return { accepted, skipped };
}

export function fileNameFromPath(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || "image";
}

export function filterComposerImagePaths(paths: readonly string[]): {
  accepted: string[];
  skipped: ComposerImageSkip[];
} {
  const accepted: string[] = [];
  const skipped: ComposerImageSkip[] = [];
  for (const path of paths) {
    const trimmed = path.trim();
    if (!trimmed) continue;
    const name = fileNameFromPath(trimmed);
    if (composerImageMimeFromName(name) === null) {
      skipped.push({ name, reason: "mime" });
      continue;
    }
    accepted.push(trimmed);
  }
  return { accepted, skipped };
}

export function collectFilesFromDataTransfer(
  data: {
    files?: ArrayLike<ComposerImageFileLike> | null;
    items?: ArrayLike<{ kind: string; getAsFile: () => ComposerImageFileLike | null }> | null;
  } | null,
): ComposerImageFileLike[] {
  if (!data) return [];
  const fromItems: ComposerImageFileLike[] = [];
  for (const item of Array.from(data.items ?? [])) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (file) fromItems.push(file);
  }
  if (fromItems.length > 0) return fromItems;
  return Array.from(data.files ?? []);
}

export function mergeComposerImageItems<T extends { id: string; path?: string }>(
  existing: readonly T[],
  incoming: readonly T[],
  max = MAX_COMPOSER_IMAGES,
): MergeComposerImagesResult<T> {
  const seen = new Set(existing.map((item) => item.path || item.id));
  const items = [...existing];
  const skipped: ComposerImageSkip[] = [];
  for (const item of incoming) {
    const key = item.path || item.id;
    if (seen.has(key)) continue;
    if (items.length >= max) {
      skipped.push({
        name: "name" in item ? String(item.name ?? "image") : "image",
        reason: "limit",
      });
      continue;
    }
    seen.add(key);
    items.push(item);
  }
  return { items, skipped };
}

export function toggleComposerImageSelected<T extends { id: string; selected: boolean }>(
  items: readonly T[],
  id: string,
): T[] {
  return items.map((item) => (item.id === id ? { ...item, selected: !item.selected } : item));
}

export function removeComposerImagesByIds<T extends { id: string }>(
  items: readonly T[],
  ids: Iterable<string>,
): T[] {
  const remove = new Set(ids);
  return items.filter((item) => !remove.has(item.id));
}

export function selectedComposerImageIds<T extends { id: string; selected: boolean }>(
  items: readonly T[],
): string[] {
  return items.filter((item) => item.selected).map((item) => item.id);
}

/** 与发送前的模型能力一致：PDF 走文本路径，图片和视频分别要求 image / video。返回文案时调用方必须保留附件。 */
export function mediaBlockedByModel(
  names: readonly string[],
  inputTypes: readonly string[],
): string | null {
  const allowed = new Set(inputTypes);
  for (const name of names) {
    const mime = composerImageMimeFromName(name);
    if (!mime) continue;
    const label = fileNameFromPath(name);
    if (mime === "video/mp4" && !allowed.has("video")) {
      return `当前模型不能接收视频，已保留附件 ${label}`;
    }
    if (mime.startsWith("image/") && !allowed.has("image")) {
      return `当前模型不能接收图片，已保留附件 ${label}`;
    }
  }
  return null;
}

export function appendComposerTrigger(draft: string, trigger: ComposerTriggerChar): string {
  if (!draft) return trigger;
  if (/\s$/.test(draft)) return `${draft}${trigger}`;
  return `${draft} ${trigger}`;
}
