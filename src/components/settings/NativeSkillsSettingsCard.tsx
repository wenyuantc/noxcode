import { useEffect, useState } from "react";
import { FolderOpen, Wrench } from "lucide-react";
import { useTranslation } from "react-i18next";

import { listNativeGlobalSkills, openNativeSkillsDir } from "@/lib/backend";
import type { NativeSkill } from "@/lib/types";
import { Button } from "@/components/ui/button";
import { SettingCard } from "./SettingCard";

export function NativeSkillsSettingsCard() {
  const { t } = useTranslation("settings");
  const [skills, setSkills] = useState<NativeSkill[]>([]);
  const [dir, setDir] = useState("");

  useEffect(() => {
    void listNativeGlobalSkills().then((doc) => {
      setSkills(doc.skills);
      setDir(doc.dir);
    });
  }, []);

  return (
    <div className="space-y-6">
      <SettingCard
        icon={Wrench}
        title={t("skills.title")}
        description={t("skills.hint")}
        badge={`${skills.length} 个全局技能`}
        headerAction={
          <Button
            variant="outline"
            size="sm"
            className="h-7 text-xs gap-1"
            onClick={() => void openNativeSkillsDir()}
          >
            <FolderOpen className="size-3.5" />
            {t("skills.openDir")}
          </Button>
        }
      >
        {skills.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-10 text-center">
            <div className="flex size-10 items-center justify-center rounded-xl border border-border/70 bg-muted/30 text-muted-foreground">
              <Wrench className="size-5" />
            </div>
            <p className="mt-3 text-xs font-semibold text-foreground">暂无全局技能</p>
            <p className="mt-1 text-[11px] text-muted-foreground max-w-sm">
              将技能规范文件（SKILL.md）存放在本地目录后，Agent 将自动感知并加载这些能力。
            </p>
            <Button
              variant="outline"
              size="sm"
              className="mt-4 h-7 text-xs gap-1"
              onClick={() => void openNativeSkillsDir()}
            >
              <FolderOpen className="size-3.5" />
              {t("skills.openDir")}
            </Button>
          </div>
        ) : (
          <div className="grid gap-2.5 sm:grid-cols-1">
            {skills.map((skill) => (
              <div
                key={skill.skill_md_path}
                className="group flex flex-col justify-between gap-1.5 rounded-xl border border-border/70 bg-card p-3.5 shadow-2xs transition-all hover:border-border hover:shadow-xs"
              >
                <div className="flex items-start gap-3">
                  <div className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg border border-border/60 bg-muted/40 text-primary">
                    <Wrench className="size-4" />
                  </div>
                  <div className="min-w-0 flex-1 space-y-1">
                    <div className="flex items-center gap-2">
                      <span className="text-xs font-semibold tracking-tight text-foreground">
                        {skill.name}
                      </span>
                      <span className="rounded-md border border-border/50 bg-background px-1.5 py-0.2 font-mono text-[10px] text-muted-foreground truncate max-w-xs">
                        {skill.skill_md_path}
                      </span>
                    </div>
                    <p className="text-[11px] text-muted-foreground leading-relaxed">
                      {skill.description}
                    </p>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}

        {dir ? (
          <div className="mt-4 flex items-center justify-between border-t border-border/40 pt-3 text-[10px] font-mono text-muted-foreground">
            <span>技能存储目录:</span>
            <span className="truncate max-w-md">{dir}</span>
          </div>
        ) : null}
      </SettingCard>
    </div>
  );
}
