import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { useNativeSteer } from "@/hooks/useNativeSteer";
import { deleteComposerImages, stageComposerImage } from "@/lib/backend";
import { COMPOSER_FILE_ACCEPT, filterComposerImageFiles } from "@/lib/composerImages";

export function PermissionSteerEditor({
  sessionId,
  label,
  onEditingChange,
  editing,
}: {
  sessionId: string;
  label: string;
  onEditingChange: (editing: boolean) => void;
  editing: boolean;
}) {
  const { t } = useTranslation("sessions");
  const steer = useNativeSteer(sessionId);
  const [text, setText] = useState("");
  const [images, setImages] = useState<Array<{ name: string; path: string }>>([]);
  const [staging, setStaging] = useState(false);
  const [imageError, setImageError] = useState<string | null>(null);
  const picker = useRef<HTMLInputElement>(null);
  const changeEditing = (value: boolean) => {
    onEditingChange(value);
  };
  const stage = async (files: File[]) => {
    if (staging || steer.busy) return;
    const filtered = filterComposerImageFiles(files);
    if (filtered.skipped.length || images.length + filtered.accepted.length > 8) {
      setImageError(t("steer.limit"));
      return;
    }
    setStaging(true);
    setImageError(null);
    const added: Array<{ name: string; path: string }> = [];
    try {
      for (const file of filtered.accepted) {
        const data = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(String(reader.result).split(",")[1]);
          reader.onerror = () => reject(reader.error);
          reader.readAsDataURL(file);
        });
        added.push({ name: file.name, path: await stageComposerImage(file.name, data) });
      }
      setImages((previous) => [...previous, ...added]);
    } catch (reason) {
      void deleteComposerImages(added.map((image) => image.path));
      setImageError(String(reason));
    } finally {
      setStaging(false);
    }
  };
  if (!steer.snapshot?.turn_id && !editing) return null;
  return (
    <div className="space-y-2 border-t border-border/50 pt-3">
      {!editing ? (
        <Button variant="outline" onClick={() => changeEditing(true)}>
          {t("steer.action")}
        </Button>
      ) : (
        <>
          <p className="text-xs font-medium">{t("steer.target", { name: label })}</p>
          <textarea
            aria-label={t("steer.editor")}
            value={text}
            disabled={steer.busy}
            onChange={(event) => setText(event.target.value)}
            onPaste={(event) => {
              if (event.clipboardData.files.length) {
                event.preventDefault();
                void stage(Array.from(event.clipboardData.files));
              }
            }}
            className="min-h-24 w-full rounded-md border bg-background p-2 text-sm"
          />
          <input
            ref={picker}
            type="file"
            multiple
            accept={COMPOSER_FILE_ACCEPT}
            className="hidden"
            aria-label={t("steer.attach")}
            onChange={(event) => {
              void stage(Array.from(event.target.files ?? []));
              event.target.value = "";
            }}
          />
          {images.map((image) => (
            <div key={image.path} className="flex items-center gap-2 text-xs">
              <span>{image.name}</span>
              <Button
                size="sm"
                variant="ghost"
                disabled={steer.busy || staging}
                aria-label={t("steer.removeImage", { name: image.name })}
                onClick={() => {
                  setImages((current) => current.filter((item) => item.path !== image.path));
                  void deleteComposerImages([image.path]);
                }}
              >
                ×
              </Button>
            </div>
          ))}
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={steer.busy || staging}
              onClick={() => picker.current?.click()}
            >
              {t("steer.attach")}
            </Button>
            <Button
              size="sm"
              disabled={
                steer.busy || staging || !steer.canSubmit || (!text.trim() && !images.length)
              }
              onClick={async () => {
                if (
                  await steer.submit(
                    text,
                    images.map((image) => image.path),
                  )
                ) {
                  setText("");
                  setImages([]);
                  changeEditing(false);
                }
              }}
            >
              {steer.busy ? t("steer.submitting") : t("steer.action")}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={steer.busy || staging}
              onClick={() => changeEditing(false)}
            >
              {t("steer.back")}
            </Button>
          </div>
          {steer.error || imageError ? (
            <p role="alert" className="text-xs text-destructive">
              {steer.error || imageError}
            </p>
          ) : null}
        </>
      )}
    </div>
  );
}
