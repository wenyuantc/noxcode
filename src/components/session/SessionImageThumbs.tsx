import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Dialog, DialogContent, DialogTitle } from "@/components/ui/dialog";
import type { NativeToolImage } from "@/lib/types";
import { cn } from "@/lib/utils";

export function SessionImageThumbs({
  images,
  className,
}: {
  images?: NativeToolImage[];
  className?: string;
}) {
  const { t } = useTranslation("sessions");
  const [preview, setPreview] = useState<NativeToolImage | null>(null);
  if (!images?.length) return null;

  return (
    <>
      <div className={cn("flex flex-wrap justify-end gap-2", className)}>
        {images.map((image, index) => (
          <button
            key={`${image.name}-${index}`}
            type="button"
            className="size-16 overflow-hidden rounded-lg border border-border/70 bg-muted/40"
            title={t("previewImage")}
            onClick={() => setPreview(image)}
          >
            <img
              src={image.data_url}
              alt={t("composerImageAlt", { name: image.name })}
              className="size-full object-cover"
            />
          </button>
        ))}
      </div>
      <Dialog open={preview !== null} onOpenChange={(open) => !open && setPreview(null)}>
        <DialogContent
          showCloseButton
          className="max-h-[90vh] max-w-4xl overflow-hidden bg-black/90 p-3 sm:max-w-4xl"
        >
          <DialogTitle className="sr-only">{preview?.name ?? t("previewImage")}</DialogTitle>
          {preview ? (
            <img
              src={preview.data_url}
              alt={t("composerImageAlt", { name: preview.name })}
              className="max-h-[80vh] w-full rounded-lg object-contain"
            />
          ) : null}
        </DialogContent>
      </Dialog>
    </>
  );
}
