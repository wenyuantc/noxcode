export const MAX_COMPOSER_IMAGES = 8;
export const MAX_COMPOSER_IMAGE_BYTES = 8 * 1024 * 1024;

export const COMPOSER_IMAGE_MIMES = ["image/png", "image/jpeg", "image/gif", "image/webp"] as const;

export const COMPOSER_IMAGE_EXTENSIONS = ["png", "jpg", "jpeg", "gif", "webp"] as const;

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
  switch (composerImageExtension(name)) {
    case "png":
      return "image/png";
    case "jpg":
    case "jpeg":
      return "image/jpeg";
    case "gif":
      return "image/gif";
    case "webp":
      return "image/webp";
    default:
      return null;
  }
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
    if (file.size > MAX_COMPOSER_IMAGE_BYTES) {
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

export function appendComposerTrigger(draft: string, trigger: ComposerTriggerChar): string {
  if (!draft) return trigger;
  if (/\s$/.test(draft)) return `${draft}${trigger}`;
  return `${draft} ${trigger}`;
}
