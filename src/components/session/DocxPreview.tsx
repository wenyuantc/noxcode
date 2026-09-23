import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { defaultOptions, renderAsync, type HElement } from "docx-preview";

import { loadPreviewBytes } from "@/lib/attachmentPreview";

const LAYOUT_STYLE = `
.docx, .docx-wrapper {
  --docx-minorEastAsia-font: "Microsoft YaHei", "微软雅黑", "PingFang SC", sans-serif;
  --docx-majorEastAsia-font: "Microsoft YaHei", "微软雅黑", "PingFang SC", sans-serif;
}
.docx-wrapper { background: #e6e6e6; }
`;

function renderNode(elem: HElement | Node | string): Node {
  if (typeof elem === "object" && elem !== null && !(elem instanceof Node)) {
    const style = elem.style;
    if (style && typeof style === "object") {
      const family = style["font-family"];
      if (typeof family === "string" && /MicrosoftYaHei/i.test(family)) {
        if (/bold/i.test(family) && !style["font-weight"]) style["font-weight"] = "700";
        style["font-family"] = '"Microsoft YaHei", "微软雅黑", "PingFang SC", sans-serif';
      }
    }
  }
  return defaultOptions.h(elem);
}

export function DocxPreview({ source }: { source: string }) {
  const { t } = useTranslation("sessions");
  const hostRef = useRef<HTMLDivElement>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    let cancelled = false;
    const shadow = host.shadowRoot ?? host.attachShadow({ mode: "open" });
    setLoading(true);
    setError(null);

    async function open() {
      const bytes = await loadPreviewBytes(source);
      if (cancelled || !host) return;
      shadow.replaceChildren();
      const mount = document.createElement("div");
      const layout = document.createElement("style");
      layout.textContent = LAYOUT_STYLE;
      shadow.append(layout, mount);
      await renderAsync(bytes.slice(), mount, mount, {
        className: "docx",
        inWrapper: true,
        ignoreWidth: false,
        ignoreHeight: false,
        ignoreFonts: false,
        breakPages: true,
        ignoreLastRenderedPageBreak: false,
        experimental: true,
        renderHeaders: true,
        renderFooters: true,
        renderFootnotes: true,
        renderEndnotes: true,
        renderComments: false,
        renderAltChunks: false,
        useBase64URL: true,
        h: renderNode,
      });
      if (cancelled) return;
      setLoading(false);
    }

    void open().catch(() => {
      if (cancelled) return;
      setLoading(false);
      setError(t("previewDocxFailed"));
    });

    return () => {
      cancelled = true;
    };
  }, [source, t]);

  return (
    <div className="relative min-h-[70vh] bg-[#e6e6e6]">
      {loading ? (
        <p className="px-4 py-6 text-sm text-neutral-600">{t("previewDocxLoading")}</p>
      ) : null}
      {error ? <p className="px-4 py-6 text-sm text-destructive">{error}</p> : null}
      <div ref={hostRef} />
    </div>
  );
}
