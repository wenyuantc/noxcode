import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { AttachmentFace } from "@/components/session/AttachmentPreviewDialog";
import {
  attachmentPreviewKind,
  decodeDataUrlBytes,
  decodeDataUrlText,
  displayAttachmentName,
  readPreviewText,
} from "./attachmentPreview";

function textDataUrl(value: string, mime = "text/html"): string {
  const bytes = new TextEncoder().encode(value);
  let binary = "";
  bytes.forEach((byte) => {
    binary += String.fromCharCode(byte);
  });
  return `data:${mime};base64,${btoa(binary)}`;
}

describe("attachment preview", () => {
  it("treats html as text and strips the staged id from the title", () => {
    const staged = "62a6064d-1b4b-4229-96ca-25128f3a024d_qwen3.8-27b-Test2.html";
    expect(attachmentPreviewKind(staged, "text/html")).toBe("text");
    expect(displayAttachmentName(staged)).toBe("qwen3.8-27b-Test2.html");
    expect(attachmentPreviewKind("shot.png")).toBe("image");
    expect(attachmentPreviewKind("notes.pdf")).toBe("pdf");
    expect(attachmentPreviewKind("clip.mp4")).toBe("video");
    expect(attachmentPreviewKind("indicator_import_template.xls")).toBe("office");
    expect(attachmentPreviewKind("说明.docx")).toBe("office");
    expect(attachmentPreviewKind("tool.bin")).toBe("file");
  });

  it("decodes utf-8 text from a data url", async () => {
    const source = textDataUrl("<h1>你好</h1>");
    expect(Array.from(decodeDataUrlBytes(source))).toEqual(
      Array.from(new TextEncoder().encode("<h1>你好</h1>")),
    );
    expect(decodeDataUrlText(source)).toBe("<h1>你好</h1>");
    await expect(readPreviewText(source)).resolves.toEqual({
      text: "<h1>你好</h1>",
      truncated: false,
    });
  });

  it("shows an html attachment as a text chip instead of a broken image", () => {
    const html = renderToStaticMarkup(
      <AttachmentFace
        name="62a6064d-1b4b-4229-96ca-25128f3a024d_qwen3.8-27b-Test2.html"
        mime="text/html"
        source="data:text/html;base64,PGgxPuaCqOWlvTwvaDE+"
      />,
    );
    expect(html).not.toContain("<img");
    expect(html).toContain("HTML");
    expect(html).toContain("qwen3.8-27b-Test2.html");
  });
});
