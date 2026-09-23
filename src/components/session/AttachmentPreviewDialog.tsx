import { isTauri } from "@tauri-apps/api/core";
import { lazy, Suspense, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import { composerImageExtension } from "@/lib/composerImages";
import {
  attachmentExtensionLabel,
  attachmentPreviewKind,
  displayAttachmentName,
  readPreviewText,
} from "@/lib/attachmentPreview";
import {
  extractAttachmentText,
  extractStagedAttachmentText,
  type OfficePreview,
} from "@/lib/backend";
import { cn } from "@/lib/utils";

const PdfPreview = lazy(() => import("./PdfPreview").then((mod) => ({ default: mod.PdfPreview })));
const DocxPreview = lazy(() =>
  import("./DocxPreview").then((mod) => ({ default: mod.DocxPreview })),
);

async function loadOfficePreview(name: string, source: string, filePath?: string) {
  if (filePath && isTauri()) {
    return extractStagedAttachmentText(filePath);
  }
  const comma = source.indexOf(",");
  if (!source.startsWith("data:") || comma < 0) {
    throw new Error("无法读取文档");
  }
  const meta = source.slice(0, comma);
  let dataBase64 = source.slice(comma + 1);
  if (!/;base64/i.test(meta)) {
    dataBase64 = btoa(decodeURIComponent(dataBase64));
  }
  return extractAttachmentText(displayAttachmentName(name), dataBase64);
}

function columnName(index: number): string {
  let n = index + 1;
  let label = "";
  while (n > 0) {
    const rem = (n - 1) % 26;
    label = String.fromCharCode(65 + rem) + label;
    n = Math.floor((n - 1) / 26);
  }
  return label;
}

function OfficePreviewBody({ preview }: { preview: OfficePreview }) {
  const [sheet, setSheet] = useState(0);
  if (preview.kind === "document") {
    return (
      <div className="min-h-[50vh] bg-neutral-200 px-4 py-6 dark:bg-neutral-950">
        <article className="mx-auto w-full max-w-2xl bg-white px-10 py-9 text-neutral-950 shadow-md ring-1 ring-black/10">
          {preview.paragraphs.map((paragraph, index) => (
            <p
              key={index}
              className="mb-3 text-[15px] leading-7 wrap-break-word whitespace-pre-wrap"
            >
              {paragraph}
            </p>
          ))}
        </article>
      </div>
    );
  }
  const current = preview.sheets[sheet] ?? preview.sheets[0];
  const width = Math.max(1, ...(current?.rows.map((row) => row.length) ?? [1]));
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {preview.sheets.length > 1 ? (
        <div className="flex gap-1 overflow-x-auto border-b border-border/60 px-3 py-2">
          {preview.sheets.map((item, index) => (
            <button
              key={item.name}
              type="button"
              className={cn(
                "shrink-0 rounded-md px-2.5 py-1 text-xs",
                index === sheet
                  ? "bg-muted font-medium text-foreground"
                  : "text-muted-foreground hover:bg-muted/60",
              )}
              onClick={() => setSheet(index)}
            >
              {item.name}
            </button>
          ))}
        </div>
      ) : null}
      <div className="min-h-0 flex-1 overflow-auto">
        <table className="min-w-full border-separate border-spacing-0 text-xs">
          <thead>
            <tr>
              <th className="sticky top-0 left-0 z-20 w-10 border-r border-b border-border/70 bg-muted px-2 py-1.5 text-center font-medium text-muted-foreground" />
              {Array.from({ length: width }, (_, index) => (
                <th
                  key={columnName(index)}
                  className="sticky top-0 z-10 border-r border-b border-border/70 bg-muted px-3 py-1.5 text-center font-medium text-muted-foreground"
                >
                  {columnName(index)}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {current?.rows.map((row, rowIndex) => (
              <tr key={`${current.name}-${rowIndex}`}>
                <th className="sticky left-0 z-10 border-r border-b border-border/60 bg-muted/70 px-2 py-1.5 text-center font-medium text-muted-foreground">
                  {rowIndex + 1}
                </th>
                {Array.from({ length: width }, (_, index) => (
                  <td
                    key={`${rowIndex}-${index}`}
                    className={cn(
                      "max-w-64 truncate border-r border-b border-border/50 px-3 py-1.5 text-foreground",
                      rowIndex === 0 && "bg-muted/40 font-medium",
                    )}
                    title={row[index] ?? ""}
                  >
                    {row[index] ?? ""}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

export function AttachmentFace({
  name,
  mime,
  source,
}: {
  name: string;
  mime?: string;
  source?: string;
}) {
  const kind = attachmentPreviewKind(name, mime);
  const label = displayAttachmentName(name);
  if (kind === "image" && source) {
    return <img src={source} alt={label} className="size-full object-cover" />;
  }
  return (
    <span className="flex size-full flex-col items-center justify-center gap-1 px-1 text-center">
      <span className="rounded-md border border-border/60 bg-background/80 px-1 py-0.5 font-mono text-[10px] font-medium text-foreground">
        {attachmentExtensionLabel(name)}
      </span>
      <span className="line-clamp-2 text-[10px] leading-tight text-muted-foreground">{label}</span>
    </span>
  );
}

export function AttachmentPreviewDialog({
  open,
  name,
  mime,
  source,
  filePath,
  onOpenChange,
}: {
  open: boolean;
  name: string;
  mime?: string;
  source: string;
  filePath?: string;
  onOpenChange: (open: boolean) => void;
}) {
  const { t } = useTranslation("sessions");
  const kind = attachmentPreviewKind(name, mime);
  const title = displayAttachmentName(name);
  const visualDocx = kind === "office" && composerImageExtension(title) === "docx";
  const [text, setText] = useState<string | null>(null);
  const [office, setOffice] = useState<OfficePreview | null>(null);
  const [truncated, setTruncated] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!open || visualDocx || (kind !== "text" && kind !== "office") || (!source && !filePath))
      return;
    let cancelled = false;
    setText(null);
    setOffice(null);
    setError(null);
    setTruncated(false);
    const pending =
      kind === "office"
        ? loadOfficePreview(name, source, filePath).then((preview) => {
            if (cancelled) return;
            setOffice(preview);
            setTruncated(preview.truncated);
          })
        : readPreviewText(source).then((result) => {
            if (cancelled) return;
            setText(result.text);
            setTruncated(result.truncated);
          });
    void pending.catch((reason: unknown) => {
      if (cancelled) return;
      if (reason instanceof Error && reason.message) setError(reason.message);
      else if (typeof reason === "string" && reason) setError(reason);
      else setError(t("previewTextFailed"));
    });
    return () => {
      cancelled = true;
    };
  }, [open, kind, visualDocx, source, filePath, name, t]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton
        className={cn(
          "flex max-h-[85vh] flex-col gap-0 overflow-hidden p-0",
          visualDocx
            ? "h-[85vh] sm:max-w-5xl"
            : kind === "text" ||
                kind === "office" ||
                kind === "image" ||
                kind === "video" ||
                kind === "pdf"
              ? "sm:max-w-3xl"
              : "sm:max-w-md",
          kind === "image" && "border-0 bg-black/90 text-white",
        )}
      >
        <div
          className={cn(
            "flex items-center gap-2 border-b px-4 py-3 pr-12",
            kind === "image" ? "border-white/15" : "border-border/60",
          )}
        >
          <DialogTitle className="min-w-0 truncate text-sm font-medium">{title}</DialogTitle>
          <span
            className={cn(
              "shrink-0 rounded-md border px-1.5 py-0.5 font-mono text-[10px]",
              kind === "image"
                ? "border-white/20 text-white/80"
                : "border-border/60 bg-muted/50 text-muted-foreground",
            )}
          >
            {attachmentExtensionLabel(name)}
          </span>
        </div>
        <div className="min-h-0 min-w-0 flex-1 overflow-auto">
          {kind === "image" && source ? (
            <img src={source} alt={title} className="max-h-[75vh] w-full object-contain" />
          ) : null}
          {kind === "video" && source ? (
            <video src={source} controls className="max-h-[75vh] w-full bg-black" />
          ) : null}
          {visualDocx && source ? (
            <Suspense
              fallback={
                <p className="bg-[#e6e6e6] px-4 py-6 text-sm text-neutral-600">
                  {t("previewDocxLoading")}
                </p>
              }
            >
              <DocxPreview source={source} />
            </Suspense>
          ) : null}
          {(kind === "office" && !visualDocx) || kind === "text" ? (
            <div className={cn("flex min-h-64 flex-col", kind === "office" && "min-h-[50vh]")}>
              {error ? <p className="px-4 py-3 text-sm text-destructive">{error}</p> : null}
              {!error && text === null && office === null ? (
                <p className="px-4 py-3 text-sm text-muted-foreground">{t("previewLoading")}</p>
              ) : null}
              {kind === "text" && text !== null ? (
                <pre className="px-4 py-3 font-mono text-xs leading-5 wrap-break-word whitespace-pre-wrap text-foreground">
                  {text}
                </pre>
              ) : null}
              {kind === "office" && office ? <OfficePreviewBody preview={office} /> : null}
              {truncated ? (
                <p className="px-4 py-3 text-xs text-muted-foreground">{t("previewTruncated")}</p>
              ) : null}
            </div>
          ) : null}
          {kind === "pdf" && source ? (
            <Suspense
              fallback={
                <p className="px-4 py-3 text-sm text-muted-foreground">{t("previewPdfLoading")}</p>
              }
            >
              <PdfPreview source={source} />
            </Suspense>
          ) : null}
          {kind === "file" || (kind === "pdf" && !source) ? (
            <p className="px-4 py-8 text-center text-sm text-muted-foreground">{title}</p>
          ) : null}
        </div>
      </DialogContent>
    </Dialog>
  );
}
