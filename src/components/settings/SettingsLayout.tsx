import { useEffect, type ComponentType } from "react";
import { useTranslation } from "react-i18next";
import { Link, useNavigate, useParams } from "react-router-dom";
import {
  ArrowLeft,
  BarChart3,
  Blocks,
  Bot,
  Brain,
  Clock,
  Database,
  Info,
  Palette,
  ShieldCheck,
  Sliders,
  Sparkles,
  Terminal,
  Workflow,
  Wrench,
  Zap,
} from "lucide-react";

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

interface SectionMeta {
  icon: ComponentType<{ className?: string }>;
  descriptionKey?: string;
}

const SECTION_META: Record<string, SectionMeta> = {
  general: { icon: Sliders, descriptionKey: "general.languageHint" },
  appearance: { icon: Palette, descriptionKey: "appearance.themeHint" },
  channels: { icon: Sparkles, descriptionKey: "channels.description" },
  ssh: { icon: Terminal, descriptionKey: "ssh.description" },
  runtime: { icon: Zap, descriptionKey: "runtime.hint" },
  permissions: { icon: ShieldCheck, descriptionKey: "permissions.description" },
  memory: { icon: Brain, descriptionKey: "memory.description" },
  automations: { icon: Clock, descriptionKey: "automations.hint" },
  subagents: { icon: Bot, descriptionKey: "subagents.description" },
  mcp: { icon: Blocks, descriptionKey: "mcp.description" },
  skills: { icon: Wrench, descriptionKey: "skills.hint" },
  hooks: { icon: Workflow, descriptionKey: "hooks.description" },
  usage: { icon: BarChart3, descriptionKey: "usage.hint" },
  database: { icon: Database, descriptionKey: "database.maintenance.description" },
  about: { icon: Info, descriptionKey: "about.description" },
};

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
  const { t } = useTranslation(["settings", "layout", "nav"]);
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

  const currentMeta = SECTION_META[current];
  const CurrentIcon = currentMeta?.icon ?? Sliders;
  const currentDescription = currentMeta?.descriptionKey
    ? t(currentMeta.descriptionKey, { defaultValue: "" })
    : "";

  return (
    <div className="flex h-screen overflow-hidden bg-background">
      {/* 侧边栏 */}
      <aside className="flex w-64 shrink-0 flex-col border-r border-sidebar-border bg-sidebar select-none">
        {/* 返回主界面 Header */}
        <div className="border-b border-sidebar-border/70 p-3">
          <Link
            to="/"
            className="group flex items-center gap-2.5 rounded-lg px-2.5 py-2 text-xs font-medium text-sidebar-foreground/90 transition-all duration-150 hover:bg-sidebar-accent/80 hover:text-sidebar-foreground active:scale-[0.99]"
          >
            <ArrowLeft className="size-4 shrink-0 text-muted-foreground transition-transform duration-150 group-hover:-translate-x-0.5 group-hover:text-sidebar-foreground" />
            <span className="flex-1 tracking-tight">{t("title")}</span>
            <span className="rounded border border-sidebar-border/80 bg-background/50 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
              Esc
            </span>
          </Link>
        </div>

        {/* 设置导航列表 */}
        <div className="min-h-0 flex-1 overflow-y-auto px-2.5 py-3 space-y-4">
          {GROUPS.map((group) => (
            <div key={group.id} className="space-y-1">
              <p className="px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground/70">
                {t(`groups.${group.id}`)}
              </p>
              {group.items.map((item) => {
                const meta = SECTION_META[item];
                const ItemIcon = meta?.icon ?? Sliders;
                const active = current === item;
                return (
                  <Link
                    key={item}
                    to={`/settings/${item}`}
                    className={cn(
                      "group flex items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-all duration-150",
                      active
                        ? "bg-sidebar-accent text-sidebar-foreground shadow-2xs font-semibold"
                        : "text-sidebar-foreground/80 hover:bg-sidebar-accent/60 hover:text-sidebar-foreground",
                    )}
                  >
                    <ItemIcon
                      className={cn(
                        "size-4 shrink-0 transition-colors",
                        active
                          ? "text-primary"
                          : "text-muted-foreground group-hover:text-sidebar-foreground",
                      )}
                    />
                    <span className="truncate tracking-tight">{t(`sections.${item}`)}</span>
                  </Link>
                );
              })}
            </div>
          ))}
        </div>

        {/* 底部品牌标语 */}
        <div className="flex items-center justify-between border-t border-sidebar-border/70 px-3.5 py-2.5">
          <span className="text-[11px] font-medium tracking-tight text-muted-foreground/60">
            noxcode
          </span>
          <span className="text-[10px] font-mono text-muted-foreground/50">v0.2</span>
        </div>
      </aside>

      {/* 主内容区 */}
      <main className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-8 py-8">
          {/* 页面头部 */}
          <header className="mb-6 flex items-start gap-3 border-b border-border/60 pb-5">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-xl border border-border/70 bg-card shadow-2xs text-primary">
              <CurrentIcon className="size-5" />
            </div>
            <div className="space-y-0.5">
              <h1 className="text-xl font-semibold tracking-tight text-foreground">
                {t(`sections.${current}`)}
              </h1>
              {currentDescription ? (
                <p className="text-xs text-muted-foreground leading-relaxed">
                  {currentDescription}
                </p>
              ) : null}
            </div>
          </header>

          {/* 模块内容 */}
          <div className="space-y-5">
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
        </div>
      </main>
    </div>
  );
}
