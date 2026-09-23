import { describe, expect, it } from "vitest";

import {
  MAX_COMPOSER_IMAGES,
  appendComposerTrigger,
  collectFilesFromDataTransfer,
  composerImageMimeFromName,
  filterComposerImageFiles,
  filterComposerImagePaths,
  isComposerImageFile,
  mediaBlockedByModel,
  mergeComposerImageItems,
  removeComposerImagesByIds,
  selectedComposerImageIds,
  toggleComposerImageSelected,
} from "./composerImages";

function file(name: string, type: string, size = 16): { name: string; type: string; size: number } {
  return { name, type, size };
}

describe("composerImageMimeFromName", () => {
  it("maps common image extensions", () => {
    expect(composerImageMimeFromName("a.PNG")).toBe("image/png");
    expect(composerImageMimeFromName("b.jpeg")).toBe("image/jpeg");
    expect(composerImageMimeFromName("c.webp")).toBe("image/webp");
    expect(composerImageMimeFromName("doc.pdf")).toBe("application/pdf");
    expect(composerImageMimeFromName("clip.mp4")).toBe("video/mp4");
    expect(composerImageMimeFromName("qwen3.8-27b-Test2.html")).toBe("text/html");
    expect(composerImageMimeFromName("indicator_import_template.xls")).toBe(
      "application/vnd.ms-excel",
    );
    expect(composerImageMimeFromName("说明.docx")).toBe(
      "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
    );
    expect(composerImageMimeFromName("note.txt")).toBe("text/plain");
    expect(composerImageMimeFromName("note.bin")).toBeNull();
  });
});

describe("mediaBlockedByModel", () => {
  it("allows pdf on text models and keeps image or video that the model cannot take", () => {
    expect(mediaBlockedByModel(["notes.pdf"], ["text"])).toBeNull();
    expect(mediaBlockedByModel(["qwen3.8-27b-Test2.html"], ["text"])).toBeNull();
    expect(mediaBlockedByModel(["indicator_import_template.xls"], ["text"])).toBeNull();
    expect(mediaBlockedByModel(["说明.doc"], ["text"])).toBeNull();
    expect(mediaBlockedByModel(["shot.png"], ["text"])).toBe(
      "当前模型不能接收图片，已保留附件 shot.png",
    );
    expect(mediaBlockedByModel(["clip.mp4"], ["text", "image"])).toBe(
      "当前模型不能接收视频，已保留附件 clip.mp4",
    );
    expect(mediaBlockedByModel(["clip.mp4", "shot.png"], ["text", "image", "video"])).toBeNull();
  });
});

describe("filterComposerImageFiles", () => {
  it("keeps allowed images and skips other types or oversized files", () => {
    const { accepted, skipped } = filterComposerImageFiles([
      file("ok.png", "image/png"),
      file("shot.jpg", "", 32),
      file("doc.pdf", "application/pdf"),
      file("clip.mp4", "video/mp4"),
      file("qwen3.8-27b-Test2.html", "text/html"),
      file("indicator_import_template.xls", "application/vnd.ms-excel"),
      file("说明.docx", ""),
      file("note.bin", ""),
      file("big.png", "image/png", 9 * 1024 * 1024),
      file("big.html", "text/html", 2 * 1024 * 1024),
    ]);
    expect(accepted.map((item) => item.name)).toEqual([
      "ok.png",
      "shot.jpg",
      "doc.pdf",
      "clip.mp4",
      "qwen3.8-27b-Test2.html",
      "indicator_import_template.xls",
      "说明.docx",
    ]);
    expect(skipped).toEqual([
      { name: "note.bin", reason: "mime" },
      { name: "big.png", reason: "size" },
      { name: "big.html", reason: "size" },
    ]);
  });

  it("accepts a file when the extension is an image even if type is empty", () => {
    expect(isComposerImageFile(file("clip.gif", ""))).toBe(true);
    expect(isComposerImageFile(file("clip", "text/plain"))).toBe(false);
  });
});

describe("collectFilesFromDataTransfer", () => {
  it("prefers clipboard items over the files list", () => {
    const files = collectFilesFromDataTransfer({
      files: [file("legacy.png", "image/png")],
      items: [
        { kind: "string", getAsFile: () => file("text", "text/plain") },
        { kind: "file", getAsFile: () => file("clip.png", "image/png") },
      ],
    });
    expect(files.map((item) => item.name)).toEqual(["clip.png"]);
  });

  it("falls back to files when items are empty", () => {
    const files = collectFilesFromDataTransfer({
      files: [file("drop.webp", "image/webp")],
      items: [],
    });
    expect(files.map((item) => item.name)).toEqual(["drop.webp"]);
  });
});

describe("filterComposerImagePaths", () => {
  it("keeps image paths and skips other files", () => {
    const { accepted, skipped } = filterComposerImagePaths([
      "/tmp/ok.png",
      String.raw`C:\Users\me\shot.JPEG`,
      "/tmp/qwen3.8-27b-Test2.html",
      "/tmp/note.bin",
      "  ",
    ]);
    expect(accepted).toEqual([
      "/tmp/ok.png",
      String.raw`C:\Users\me\shot.JPEG`,
      "/tmp/qwen3.8-27b-Test2.html",
    ]);
    expect(skipped).toEqual([{ name: "note.bin", reason: "mime" }]);
  });
});

describe("merge and selection", () => {
  it("dedupes by path, caps at eight, and supports checkbox delete", () => {
    const existing = Array.from({ length: 7 }, (_, index) => ({
      id: `e${index}`,
      path: `/tmp/e${index}.png`,
      selected: index === 1,
    }));
    const incoming = [
      { id: "dup", path: "/tmp/e0.png", selected: false },
      { id: "n1", path: "/tmp/n1.png", selected: true },
      { id: "n2", path: "/tmp/n2.png", selected: false },
    ];
    const merged = mergeComposerImageItems(existing, incoming);
    expect(merged.items).toHaveLength(MAX_COMPOSER_IMAGES);
    expect(merged.items[merged.items.length - 1]?.id).toBe("n1");
    expect(merged.skipped).toEqual([{ name: "image", reason: "limit" }]);

    const toggled = toggleComposerImageSelected(merged.items, "e0");
    expect(selectedComposerImageIds(toggled)).toEqual(["e0", "e1", "n1"]);
    const removed = removeComposerImagesByIds(toggled, selectedComposerImageIds(toggled));
    expect(removed.map((item) => item.id)).toEqual(["e2", "e3", "e4", "e5", "e6"]);
  });
});

describe("appendComposerTrigger", () => {
  it("inserts @ / $ without wiping existing text", () => {
    expect(appendComposerTrigger("", "@")).toBe("@");
    expect(appendComposerTrigger("hello", "/")).toBe("hello /");
    expect(appendComposerTrigger("hello ", "$")).toBe("hello $");
  });
});
