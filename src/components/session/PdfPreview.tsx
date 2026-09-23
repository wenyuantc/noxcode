import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { getDocument, GlobalWorkerOptions, type PDFDocumentProxy } from "pdfjs-dist";
import workerUrl from "pdfjs-dist/build/pdf.worker.min.mjs?url";

import { loadPreviewBytes } from "@/lib/attachmentPreview";

GlobalWorkerOptions.workerSrc = workerUrl;

const MAX_PAGES = 20;

const assetBase = `${import.meta.env.BASE_URL}pdfjs`;

function pdfFailureKey(reason: unknown): "previewPdfPassword" | "previewPdfFailed" {
  if (
    reason &&
    typeof reason === "object" &&
    "name" in reason &&
    reason.name === "PasswordException"
  ) {
    return "previewPdfPassword";
  }
  return "previewPdfFailed";
}

export function PdfPreview({ source }: { source: string }) {
  const { t } = useTranslation("sessions");
  const frameRef = useRef<HTMLDivElement>(null);
  const canvases = useRef<Array<HTMLCanvasElement | null>>([]);
  const docRef = useRef<PDFDocumentProxy | null>(null);
  const [pageCount, setPageCount] = useState(0);
  const [aspects, setAspects] = useState<number[]>([]);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    setPageCount(0);
    setAspects([]);
    setTruncated(false);
    canvases.current = [];

    let task: ReturnType<typeof getDocument> | null = null;
    async function open() {
      const bytes = await loadPreviewBytes(source);
      if (cancelled) return;
      task = getDocument({
        data: bytes.slice(),
        cMapUrl: `${assetBase}/cmaps/`,
        cMapPacked: true,
        standardFontDataUrl: `${assetBase}/standard_fonts/`,
        wasmUrl: `${assetBase}/wasm/`,
        useSystemFonts: true,
      });
      try {
        const doc = await task.promise;
        task = null;
        if (cancelled || doc.numPages < 1) {
          await doc.destroy();
          if (!cancelled) {
            setLoading(false);
            setError(t("previewPdfFailed"));
          }
          return;
        }
        const count = Math.min(doc.numPages, MAX_PAGES);
        const ratios: number[] = [];
        for (let number = 1; number <= count; number += 1) {
          if (cancelled) {
            await doc.destroy();
            return;
          }
          const page = await doc.getPage(number);
          const base = page.getViewport({ scale: 1 });
          ratios.push(base.width / base.height);
          page.cleanup();
        }
        if (cancelled) {
          await doc.destroy();
          return;
        }
        docRef.current = doc;
        setAspects(ratios);
        setTruncated(doc.numPages > MAX_PAGES);
        setPageCount(count);
        setLoading(false);
      } catch (reason: unknown) {
        if (cancelled) return;
        setLoading(false);
        setError(t(pdfFailureKey(reason)));
      }
    }

    void open().catch((reason: unknown) => {
      if (cancelled) return;
      setLoading(false);
      setError(t(pdfFailureKey(reason)));
    });

    return () => {
      cancelled = true;
      const doc = docRef.current;
      docRef.current = null;
      if (doc) void doc.destroy();
      else void task?.destroy();
    };
  }, [source, t]);

  useEffect(() => {
    const doc = docRef.current;
    const frame = frameRef.current;
    if (!doc || pageCount === 0 || !frame) return;
    let cancelled = false;
    const renders: Array<{ cancel: () => void }> = [];
    const width = Math.max(280, frame.clientWidth - 32);
    const pixelRatio = Math.min(window.devicePixelRatio || 1, 2);

    async function paint() {
      for (let index = 0; index < pageCount; index += 1) {
        if (cancelled) return;
        const canvas = canvases.current[index];
        if (!canvas) continue;
        const page = await doc.getPage(index + 1);
        if (cancelled) return;
        const base = page.getViewport({ scale: 1 });
        const cssScale = width / base.width;
        const viewport = page.getViewport({ scale: cssScale * pixelRatio });
        canvas.width = Math.ceil(viewport.width);
        canvas.height = Math.ceil(viewport.height);
        const render = page.render({ canvas, viewport });
        renders.push(render);
        await render.promise;
        page.cleanup();
      }
    }

    void paint().catch((reason: unknown) => {
      if (cancelled) return;
      setError(t(pdfFailureKey(reason)));
    });

    return () => {
      cancelled = true;
      for (const render of renders) render.cancel();
    };
  }, [pageCount, source, t]);

  return (
    <div
      ref={frameRef}
      className="flex min-h-[50vh] w-full flex-col gap-4 bg-neutral-200 px-4 py-4 dark:bg-neutral-950"
      aria-busy={loading}
    >
      {loading ? (
        <p className="py-6 text-sm text-muted-foreground">{t("previewPdfLoading")}</p>
      ) : null}
      {error ? <p className="py-6 text-sm text-destructive">{error}</p> : null}
      {Array.from({ length: pageCount }, (_, index) => (
        <div key={index} className="w-full" style={{ aspectRatio: aspects[index] ?? 1.414 }}>
          <canvas
            ref={(node) => {
              canvases.current[index] = node;
            }}
            aria-label={t("previewPdfPage", { page: index + 1 })}
            className="block h-full w-full bg-white shadow-md"
          />
        </div>
      ))}
      {truncated ? (
        <p className="pb-2 text-xs text-muted-foreground">
          {t("previewPdfTruncated", { count: MAX_PAGES })}
        </p>
      ) : null}
    </div>
  );
}
