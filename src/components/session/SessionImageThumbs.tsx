import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { NativeToolImage } from "@/lib/types";
import { cn } from "@/lib/utils";
import { AttachmentFace, AttachmentPreviewDialog } from "./AttachmentPreviewDialog";

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
            <AttachmentFace name={image.name} mime={image.mime_type} source={image.data_url} />
          </button>
        ))}
      </div>
      <AttachmentPreviewDialog
        open={preview !== null}
        name={preview?.name ?? ""}
        mime={preview?.mime_type}
        source={preview?.data_url ?? ""}
        onOpenChange={(open) => {
          if (!open) setPreview(null);
        }}
      />
    </>
  );
}
