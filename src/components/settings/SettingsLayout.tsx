import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { Link, useNavigate, useParams } from "react-router-dom";

import { cn } from "@/lib/utils";
import { useChannelStore } from "@/stores/channelStore";
import { useSettingsStore } from "@/stores/settingsStore";
import { AboutSection } from "./AboutSection";
import { AppearanceSection } from "./AppearanceSection";
import { AutomationsSection } from "./AutomationsSection";
import { DatabaseSection } from "./DatabaseSection";
import { GeneralSection } from "./GeneralSection";
import { AiChannelsSettingsTab } from "./AiChannelsSettingsTab";
import { McpSettingsTab } from "./McpSettingsTab";
import { MemorySection } from "./MemorySection";
import { NativeHooksSettingsCard } from "./NativeHooksSettingsCard";
import { NativeRuntimeSection } from "./NativeRuntimeSection";
import { NativeSkillsSettingsCard } from "./NativeSkillsSettingsCard";
import { PermissionRulesSection } from "./PermissionRulesSection";
import { SshSettingsSection } from "./SshSettingsSection";
import { SubagentsSettingsTab } from "./SubagentsSettingsTab";
import { UsageSection } from "./UsageSection";

const GROUPS = [
  {
    id: "basic",
    items: ["general", "appearance", "channels", "ssh"],
  },
  {
    id: "agent",
    items: [
      "runtime",
      "permissions",
      "memory",
      "automations",
      "subagents",
      "mcp",
      "skills",
      "hooks",
    ],
  },
  {
    id: "data",
    items: ["usage", "database", "about"],
  },
] as const;

export function SettingsLayout() {
  const { t } = useTranslation("settings");
  const { section } = useParams();
  const navigate = useNavigate();
  const current = section ?? "general";

  useEffect(() => {
    void Promise.all([useSettingsStore.getState().load(), useChannelStore.getState().load()]);
  }, []);

  useEffect(() => {
    const known = GROUPS.flatMap((group) => group.items) as readonly string[];
    if (!section || !known.includes(section)) {
      void navigate("/settings/general", { replace: true });
    }
  }, [navigate, section]);

  return (
    <div className="flex h-screen bg-background">
      <aside className="w-64 shrink-0 border-r bg-sidebar">
        <Link
          to="/"
          className="block px-4 py-3 text-sm text-muted-foreground hover:text-foreground"
        >
          ← {t("title")}
        </Link>
        {GROUPS.map((group) => (
          <div key={group.id} className="px-3 py-2">
            <p className="px-2 pb-1 text-[11px] font-medium text-muted-foreground">
              {t(`groups.${group.id}`)}
            </p>
            {group.items.map((item) => (
              <Link
                key={item}
                to={`/settings/${item}`}
                className={cn(
                  "block rounded-md px-2 py-1.5 text-sm hover:bg-sidebar-accent",
                  current === item && "bg-sidebar-accent",
                )}
              >
                {t(`sections.${item}`)}
              </Link>
            ))}
          </div>
        ))}
      </aside>
      <main className="min-w-0 flex-1 overflow-y-auto p-8">
        <div className="mx-auto max-w-3xl space-y-4">
          <h1 className="text-xl font-medium">{t(`sections.${current}`)}</h1>
          {current === "general" ? <GeneralSection /> : null}
          {current === "appearance" ? <AppearanceSection /> : null}
          {current === "channels" ? <AiChannelsSettingsTab /> : null}
          {current === "ssh" ? <SshSettingsSection /> : null}
          {current === "runtime" ? <NativeRuntimeSection /> : null}
          {current === "permissions" ? <PermissionRulesSection /> : null}
          {current === "memory" ? <MemorySection /> : null}
          {current === "automations" ? <AutomationsSection /> : null}
          {current === "subagents" ? <SubagentsSettingsTab /> : null}
          {current === "mcp" ? <McpSettingsTab /> : null}
          {current === "skills" ? <NativeSkillsSettingsCard /> : null}
          {current === "hooks" ? <NativeHooksSettingsCard /> : null}
          {current === "usage" ? <UsageSection /> : null}
          {current === "database" ? <DatabaseSection /> : null}
          {current === "about" ? <AboutSection /> : null}
        </div>
      </main>
    </div>
  );
}
