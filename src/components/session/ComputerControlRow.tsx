import {
  Camera,
  ChevronsUpDown,
  Clock,
  Keyboard,
  Layers,
  Monitor,
  MousePointerClick,
  Move,
  Sliders,
  X,
  ZoomIn,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { CodeBlock } from "@/components/code/CodeBlock";
import type { GroupedSessionItem, NativeToolImage } from "@/lib/sessionLines";
import {
  parseComputerStep,
  summarizeComputerActions,
  type ComputerActionType,
  type ParsedComputerStep,
} from "@/lib/sessionLines";
import { cn } from "@/lib/utils";
import { SegmentCard } from "./SegmentCard";

function actionIcon(actionType: ComputerActionType): ReactNode {
  switch (actionType) {
    case "screenshot":
      return <Camera className="size-3.5" />;
    case "click":
      return <MousePointerClick className="size-3.5" />;
    case "type":
    case "keypress":
      return <Keyboard className="size-3.5" />;
    case "wait":
      return <Clock className="size-3.5" />;
    case "scroll":
      return <ChevronsUpDown className="size-3.5" />;
    case "drag":
      return <Move className="size-3.5" />;
    case "list_apps":
      return <Layers className="size-3.5" />;
    default:
      return <Sliders className="size-3.5" />;
  }
}

function formatDuration(ms?: number): string | null {
  if (ms == null) return null;
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

interface LightboxImage {
  url: string;
  name: string;
  title: string;
}

export function ComputerControlRow({
  items,
  running = false,
}: {
  items: GroupedSessionItem[];
  running?: boolean;
}) {
  const { t } = useTranslation("sessions");
  const summary = summarizeComputerActions(items, running);
  const [userToggled, setUserToggled] = useState<boolean | null>(null);
  const prevRunningRef = useRef(running);
  const [previewImage, setPreviewImage] = useState<LightboxImage | null>(null);

  // Auto-expand on running, auto-collapse when done, reset manual override on transition
  useEffect(() => {
    if (prevRunningRef.current !== running) {
      setUserToggled(null);
      prevRunningRef.current = running;
    }
  }, [running]);

  // Handle ESC to close lightbox
  useEffect(() => {
    if (!previewImage) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setPreviewImage(null);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [previewImage]);

  const open = userToggled ?? Boolean(running);

  const titleText = running
    ? summary.lastAction
      ? t("computerRunningStep", { action: summary.lastAction })
      : t("computerRunning")
    : summary.app && summary.actions.length > 0
      ? t("computerStepsWithApp", {
          app: summary.app,
          count: summary.count,
          actions: summary.actions.join(" · "),
        })
      : summary.actions.length > 0
        ? t("computerStepsWithActions", {
            count: summary.count,
            actions: summary.actions.join(" · "),
          })
        : t("computerSteps", { count: summary.count });

  const steps = items.map(parseComputerStep);
  const hasFailed = steps.some((s) => s.failed);

  return (
    <>
      <SegmentCard
        icon={<Monitor className="size-3.5" />}
        iconClassName="bg-purple-500/15 text-purple-600 dark:text-purple-400"
        badge={t("computer")}
        title={titleText}
        running={running}
        tone={hasFailed ? "danger" : "default"}
        open={open}
        onOpenChange={(next) => setUserToggled(next)}
        contentClassName="space-y-2.5 p-3"
      >
        <div className="space-y-2.5">
          {steps.map((step, index) => (
            <ComputerStepCard
              key={`${items[index]?.id ?? index}-${step.label}`}
              step={step}
              index={index}
              onPreviewImage={(img) => setPreviewImage(img)}
            />
          ))}
        </div>
      </SegmentCard>

      {/* Lightbox Modal */}
      {previewImage ? (
        <div
          role="dialog"
          aria-modal="true"
          className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 backdrop-blur-sm p-4 animate-in fade-in duration-150"
          onClick={() => setPreviewImage(null)}
        >
          <div
            className="relative flex flex-col max-h-[90vh] max-w-[92vw] overflow-hidden rounded-xl border border-white/10 bg-background/95 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="flex items-center justify-between border-b border-border/50 px-4 py-2.5 bg-muted/30">
              <div className="flex items-center gap-2 min-w-0 pr-4">
                <Camera className="size-4 text-purple-500 shrink-0" />
                <span className="truncate text-xs font-medium text-foreground">
                  {previewImage.title || previewImage.name || t("screenshotPreview")}
                </span>
              </div>
              <button
                type="button"
                onClick={() => setPreviewImage(null)}
                className="rounded-md p-1 text-muted-foreground hover:bg-muted hover:text-foreground transition-colors cursor-pointer"
                title={t("closePreview")}
                aria-label={t("closePreview")}
              >
                <X className="size-4" />
              </button>
            </div>
            <div className="flex-1 overflow-auto p-3 flex items-center justify-center bg-black/40">
              <img
                src={previewImage.url}
                alt={previewImage.name || t("screenshotPreview")}
                className="max-h-[80vh] max-w-full rounded-md object-contain select-none shadow-lg"
              />
            </div>
          </div>
        </div>
      ) : null}
    </>
  );
}

function ComputerStepCard({
  step,
  index,
  onPreviewImage,
}: {
  step: ParsedComputerStep;
  index: number;
  onPreviewImage: (image: LightboxImage) => void;
}) {
  const { t } = useTranslation("sessions");
  const [detailsOpen, setDetailsOpen] = useState(false);
  const duration = formatDuration(step.durationMs);

  return (
    <div
      className={cn(
        "rounded-lg border bg-muted/20 p-2.5 transition-colors text-xs",
        step.failed ? "border-rose-500/30 bg-rose-500/5" : "border-border/50",
      )}
    >
      <div className="flex items-center justify-between gap-2 min-w-0">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          <span className="flex size-5 shrink-0 items-center justify-center rounded bg-purple-500/10 text-purple-600 dark:text-purple-400 font-mono text-[10px] font-semibold">
            {index + 1}
          </span>
          <span className="text-muted-foreground/80 shrink-0">{actionIcon(step.actionType)}</span>
          <span
            className={cn(
              "font-mono text-code-sm truncate font-medium",
              step.failed ? "text-rose-600 dark:text-rose-400" : "text-foreground/90",
            )}
            title={step.label}
          >
            {step.label}
          </span>
        </div>

        <div className="flex items-center gap-2 shrink-0">
          {duration ? (
            <span className="text-badge font-mono text-muted-foreground/70 bg-muted/40 px-1.5 py-0.5 rounded border border-border/30">
              {duration}
            </span>
          ) : null}
          {step.failed ? (
            <span className="rounded border border-red-500/30 bg-red-500/10 px-1.5 py-0.5 text-badge font-medium text-red-600 dark:text-red-400">
              {t("toolFailed")}
            </span>
          ) : null}
        </div>
      </div>

      {/* Screenshot Thumbnail Strip */}
      {step.images.length > 0 ? (
        <div className="mt-2.5 flex flex-wrap gap-2">
          {step.images.map((image: NativeToolImage, imgIdx) => (
            <button
              key={`${image.name}-${imgIdx}`}
              type="button"
              onClick={() =>
                onPreviewImage({
                  url: image.data_url,
                  name: image.name,
                  title: `${step.label} (${image.name})`,
                })
              }
              className="group relative block overflow-hidden rounded-md border border-border/60 bg-black/10 cursor-pointer focus:outline-none focus:ring-1 focus:ring-purple-500"
              title={t("screenshotPreview")}
            >
              <img
                src={image.data_url}
                alt={t("toolImageAlt", { name: image.name })}
                className="h-24 w-auto max-w-xs object-cover transition-transform duration-150 group-hover:scale-[1.02]"
              />
              <div className="absolute inset-0 flex items-center justify-center bg-black/40 opacity-0 transition-opacity group-hover:opacity-100">
                <span className="inline-flex items-center gap-1 rounded bg-black/60 px-2 py-1 text-[11px] font-medium text-white shadow">
                  <ZoomIn className="size-3" />
                  {t("screenshotPreview")}
                </span>
              </div>
            </button>
          ))}
        </div>
      ) : null}

      {/* Optional Raw Details / Accessibility Tree Toggle */}
      {step.result ? (
        <div className="mt-2 pt-1 border-t border-border/30">
          <button
            type="button"
            className="flex items-center gap-1 text-[11px] text-muted-foreground/70 hover:text-foreground transition-colors cursor-pointer py-0.5"
            onClick={() => setDetailsOpen((prev) => !prev)}
          >
            <span className="font-mono">{detailsOpen ? "▾" : "▸"}</span>
            <span>{t("computerStepDetails")}</span>
          </button>
          {detailsOpen ? (
            <div className="mt-1">
              <CodeBlock className="max-h-60" code={step.result} language="text" />
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
