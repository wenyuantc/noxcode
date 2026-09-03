import { useEffect, useState } from "react";
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
    <SettingCard title={t("skills.title")} description={t("skills.hint")} badge={dir}>
      <div className="space-y-2">
        {skills.map((skill) => (
          <div key={skill.skill_md_path} className="rounded-md border px-3 py-2 text-sm">
            <p className="font-medium">{skill.name}</p>
            <p className="text-xs text-muted-foreground">{skill.description}</p>
          </div>
        ))}
        <Button variant="outline" onClick={() => void openNativeSkillsDir()}>
          {t("skills.openDir")}
        </Button>
      </div>
    </SettingCard>
  );
}
