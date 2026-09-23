import {
  composerImageExtension,
  composerImageMimeFromName,
  fileNameFromPath,
} from "@/lib/composerImages";

export type AttachmentPreviewKind = "image" | "video" | "pdf" | "text" | "office" | "file";

const OFFICE_EXTENSIONS = new Set(["xls", "xlsx", "doc", "docx"]);

export const PREVIEW_TEXT_CHARS = 100_000;

const STAGED_PREFIX = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}_/i;

export function attachmentPreviewKind(name: string, mime = ""): AttachmentPreviewKind {
  const extension = composerImageExtension(displayAttachmentName(name));
  if (extension && OFFICE_EXTENSIONS.has(extension)) return "office";
  const kind = mime.trim().toLowerCase() || composerImageMimeFromName(name) || "";
  if (kind.startsWith("image/")) return "image";
  if (kind.startsWith("video/")) return "video";
  if (kind === "application/pdf") return "pdf";
  if (kind.startsWith("text/") || kind === "application/json" || kind === "application/xml") {
    return "text";
  }
  return "file";
}

export function displayAttachmentName(name: string): string {
  const base = fileNameFromPath(name);
  const stripped = base.replace(STAGED_PREFIX, "");
  return stripped || base;
}

export function attachmentExtensionLabel(name: string): string {
  return (composerImageExtension(displayAttachmentName(name)) ?? "file").toUpperCase();
}

export function decodeDataUrlBytes(source: string): Uint8Array {
  const comma = source.indexOf(",");
  if (!source.startsWith("data:") || comma < 0) throw new Error("data url");
  const meta = source.slice(0, comma);
  const payload = source.slice(comma + 1);
  if (!/;base64/i.test(meta)) return new TextEncoder().encode(decodeURIComponent(payload));
  const binary = atob(payload);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

export function decodeDataUrlText(source: string): string {
  return new TextDecoder("utf-8", { fatal: true }).decode(decodeDataUrlBytes(source));
}

export async function loadPreviewBytes(source: string): Promise<Uint8Array> {
  if (source.startsWith("data:")) return decodeDataUrlBytes(source);
  const response = await fetch(source);
  if (!response.ok) throw new Error(String(response.status));
  return new Uint8Array(await response.arrayBuffer());
}

export async function readPreviewText(
  source: string,
): Promise<{ text: string; truncated: boolean }> {
  const raw = source.startsWith("data:") ? decodeDataUrlText(source) : await fetchText(source);
  if (raw.length <= PREVIEW_TEXT_CHARS) return { text: raw, truncated: false };
  return { text: raw.slice(0, PREVIEW_TEXT_CHARS), truncated: true };
}

async function fetchText(source: string): Promise<string> {
  const response = await fetch(source);
  if (!response.ok) throw new Error(String(response.status));
  return response.text();
}
