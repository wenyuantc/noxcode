import { X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import type { ComposerImageItem } from "@/lib/composerImages";
import { selectedComposerImageIds } from "@/lib/composerImages";

interface ComposerImageStripProps {
  images: ComposerImageItem[];
  onToggle: (id: string) => void;
  onRemove: (id: string) => void;
  onRemoveSelected: () => void;
}

export function ComposerImageStrip({
  images,
  onToggle,
  onRemove,
  onRemoveSelected,
}: ComposerImageStripProps) {
  const { t } = useTranslation("sessions");
  const [previewId, setPreviewId] = useState<string | null>(null);
  const preview = images.find((image) => image.id === previewId) ?? null;
  const hasSelected = selectedComposerImageIds(images).length > 0;

  if (images.length === 0) return null;

  return (
    <div className="flex flex-wrap items-end gap-2 px-4 pt-3">
      {images.map((image) => (
        <div key={image.id} className="relative size-16">
          <button
            type="button"
            className="size-16 overflow-hidden rounded-lg border border-border/70 bg-muted/40"
            title={t("previewImage")}
            onClick={() => setPreviewId(image.id)}
          >
            <img
              src={image.previewUrl}
              alt={t("composerImageAlt", { name: image.name })}
              className="size-full object-cover"
            />
          </button>
          <Checkbox
            checked={image.selected}
            aria-label={t("selectImage", { name: image.name })}
            className="absolute bottom-1 left-1 z-10 size-3.5 border-white/80 bg-black/55"
            onClick={(event) => event.stopPropagation()}
            onCheckedChange={() => onToggle(image.id)}
          />
          <button
            type="button"
            className="absolute -top-1.5 -right-1.5 z-10 inline-flex size-5 items-center justify-center rounded-full bg-white text-black shadow-sm"
            title={t("removeImage")}
            aria-label={t("removeImage")}
            onClick={() => onRemove(image.id)}
          >
            <X className="size-3" />
          </button>
        </div>
      ))}
      {hasSelected ? (
        <Button
          type="button"
          size="xs"
          variant="outline"
          className="h-6 text-xs"
          onClick={onRemoveSelected}
        >
          {t("removeSelectedImages")}
        </Button>
      ) : null}
      <Dialog open={preview !== null} onOpenChange={(open) => !open && setPreviewId(null)}>
        <DialogContent
          showCloseButton
          className="max-h-[90vh] max-w-4xl overflow-hidden bg-black/90 p-3 sm:max-w-4xl"
        >
          <DialogTitle className="sr-only">{preview?.name ?? t("previewImage")}</DialogTitle>
          {preview ? (
            <img
              src={preview.previewUrl}
              alt={t("composerImageAlt", { name: preview.name })}
              className="max-h-[80vh] w-full rounded-lg object-contain"
            />
          ) : null}
        </DialogContent>
      </Dialog>
    </div>
  );
}
