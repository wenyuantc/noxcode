import { Toast as ToastPrimitive } from "@base-ui/react/toast";
import { AlertCircle, AlertTriangle, CheckCircle2, Info, Loader2, X } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import {
  limitedToastCount,
  TOAST_LIMIT,
  TOAST_LIMIT_EXPANDED,
  toastManager,
  type ToastVariant,
} from "@/lib/toast";
import { cn } from "@/lib/utils";

interface VariantConfig {
  icon: LucideIcon;
  iconClassName: string;
  frameClassName: string;
}

const VARIANT_CONFIGS: Record<ToastVariant, VariantConfig> = {
  success: {
    icon: CheckCircle2,
    iconClassName: "text-emerald-600 dark:text-emerald-400",
    frameClassName: "border-emerald-500/30",
  },
  info: {
    icon: Info,
    iconClassName: "text-muted-foreground",
    frameClassName: "border-border/80",
  },
  warning: {
    icon: AlertTriangle,
    iconClassName: "text-amber-600 dark:text-amber-400",
    frameClassName: "border-amber-500/30",
  },
  error: {
    icon: AlertCircle,
    iconClassName: "text-destructive",
    frameClassName: "border-destructive/30",
  },
  loading: {
    icon: Loader2,
    iconClassName: "animate-spin text-primary",
    frameClassName: "border-primary/30",
  },
};

/** 把 manager 的 `type` 收敛到已知变体，未知类型按 info 展示。 */
export function resolveVariant(type: string | undefined): ToastVariant {
  if (type === "success" || type === "warning" || type === "error" || type === "loading") {
    return type;
  }
  return "info";
}

/** `Toaster` 通过它请求「展开全部」：把 Provider 的 limit 放宽到能容纳当前所有通知。 */
interface ToastStackContextValue {
  setExpanded: (expanded: boolean) => void;
}

const ToastStackContext = createContext<ToastStackContextValue | null>(null);

const noopSetExpanded = () => {};

function useToastStackContext(): ToastStackContextValue {
  return useContext(ToastStackContext) ?? { setExpanded: noopSetExpanded };
}

/**
 * 全局唯一的 toast 宿主上下文：App.tsx 挂载一次。
 * 通过共享 manager 让业务代码直接调用 toast API，无需中间 state 转发。
 *
 * 默认仍是 `TOAST_LIMIT` 条（最新的优先展示，其余折叠）；
 * 只有用户在 `Toaster` 里主动展开时才放宽 limit，避免持久 error 被静默隐藏。
 */
export function ToastProvider({ children }: { children?: ReactNode }) {
  const [expanded, setExpandedState] = useState(false);
  const setExpanded = useCallback((value: boolean) => setExpandedState(value), []);
  const stackContext = useMemo(() => ({ setExpanded }), [setExpanded]);

  return (
    <ToastStackContext.Provider value={stackContext}>
      <ToastPrimitive.Provider
        toastManager={toastManager}
        limit={expanded ? TOAST_LIMIT_EXPANDED : TOAST_LIMIT}
        timeout={3000}
      >
        {children}
      </ToastPrimitive.Provider>
    </ToastStackContext.Provider>
  );
}

export interface ToastItemProps {
  toast: ToastPrimitive.Root.ToastObject;
}

/**
 * 单条 toast：只有关闭按钮能关闭（整卡点击会误触，长文本还要能选中复制）；
 * 正文限高并可滚动，样式使用语义色板，明暗主题下都可读。
 *
 * 没有 title 时根节点会缺 accessible name，这里用 `notifications` 兜底；
 * 正文高度按视口高度自适应（dvh），短屏下卡片更矮，配合外层可滚动栈保证可达。
 */
export function ToastItem({ toast }: ToastItemProps) {
  const { t } = useTranslation("common");
  const variant = resolveVariant(toast.type);
  const config = VARIANT_CONFIGS[variant];
  const Icon = config.icon;
  const hasTitle = Boolean(toast.title);

  return (
    <ToastPrimitive.Root
      toast={toast}
      swipeDirection={[]}
      aria-label={hasTitle ? undefined : t("notifications")}
      className={cn(
        "pointer-events-auto flex w-fit max-w-[min(32rem,calc(100vw-2rem))] items-start gap-2.5 rounded-xl border bg-popover px-3.5 py-2.5 text-popover-foreground shadow-lg ring-1 ring-foreground/5",
        "data-starting-style:animate-in data-starting-style:fade-in-0 data-starting-style:zoom-in-95 data-ending-style:animate-out data-ending-style:fade-out-0 data-ending-style:zoom-out-95",
        "data-limited:hidden",
        config.frameClassName,
      )}
    >
      <Icon aria-hidden className={cn("mt-0.5 size-4 shrink-0", config.iconClassName)} />
      <div className="min-w-0 flex-1 space-y-0.5">
        {toast.title ? (
          <ToastPrimitive.Title className="text-sm leading-tight font-medium break-words">
            {toast.title}
          </ToastPrimitive.Title>
        ) : null}
        {toast.description ? (
          <ToastPrimitive.Description className="max-h-[min(30dvh,12rem)] overflow-y-auto text-xs leading-relaxed break-words whitespace-pre-wrap select-text opacity-90">
            {toast.description}
          </ToastPrimitive.Description>
        ) : null}
      </div>
      <ToastPrimitive.Close
        aria-label={t("close")}
        className="-mt-1 -mr-1 shrink-0 rounded-md p-1 opacity-70 transition-opacity hover:opacity-100 focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
      >
        <X aria-hidden className="size-3.5" />
      </ToastPrimitive.Close>
    </ToastPrimitive.Root>
  );
}

export interface ToastOverflowButtonProps {
  /** 被 limit 折叠、当前不可见的通知条数。 */
  hiddenCount: number;
  /** 展开全部通知（放宽 Provider 的 limit）。 */
  onExpand: () => void;
}

/** 「还有 N 条通知」入口：默认 limit 之外的持久 error 靠它才不会被静默丢掉。 */
export function ToastOverflowButton({ hiddenCount, onExpand }: ToastOverflowButtonProps) {
  const { t } = useTranslation("common");

  return (
    <button
      type="button"
      onClick={onExpand}
      className="shrink-0 rounded-full border border-border/80 bg-popover px-3 py-1 text-xs text-popover-foreground shadow-sm transition-colors hover:bg-accent focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
    >
      {t("toastMoreHidden", { hidden: hiddenCount })}
    </button>
  );
}

/**
 * 外层 Viewport：全宽、透明、`pointer-events-none`，不占布局也不拦截页面点击；
 * 这里刻意不加 `overflow-hidden`，短屏裁剪交给内层滚动容器，避免卡片被裁掉又滚不到。
 */
export const TOAST_VIEWPORT_CLASS =
  "pointer-events-none fixed inset-x-0 top-3 z-[200] flex max-h-[calc(100dvh-1.5rem)] flex-col items-center px-4";

/**
 * 内层栈容器：宽度贴合卡片、限高且 `overflow-y-auto`，短屏下每张卡片和关闭按钮都能滚到；
 * `pointer-events-auto` 让滚动条与关闭按钮可交互，`overscroll-contain` 防止滚到底后带动页面。
 */
export const TOAST_STACK_CLASS =
  "pointer-events-auto flex max-h-[calc(100dvh-1.5rem)] min-h-0 w-fit max-w-full flex-col items-center gap-2 overflow-y-auto overscroll-contain";

/**
 * 顶部居中的浮动 toast 栈：
 * - 外容器全宽且 `pointer-events-none`，不占布局、不拦截页面点击；
 * - 内层栈宽度贴合卡片、`pointer-events-auto`，并用 `overflow-y-auto` 限高滚动，
 *   短屏（如 420x480）下每张卡片和关闭按钮都能滚动到；
 * - 超过 TOAST_LIMIT 的通知被折叠为 `data-limited:hidden`，
 *   由「还有 N 条通知」按钮展开（展开时 Provider limit 放宽，全部可见）。
 */
export function Toaster() {
  const { toasts } = ToastPrimitive.useToastManager();
  const { setExpanded } = useToastStackContext();
  const { t } = useTranslation("common");
  const hiddenCount = limitedToastCount(toasts);

  useEffect(() => {
    if (toasts.length === 0) {
      setExpanded(false);
    }
  }, [toasts.length, setExpanded]);

  return (
    <ToastPrimitive.Portal>
      <ToastPrimitive.Viewport aria-label={t("notifications")} className={TOAST_VIEWPORT_CLASS}>
        <div className={TOAST_STACK_CLASS}>
          {hiddenCount > 0 ? (
            <ToastOverflowButton hiddenCount={hiddenCount} onExpand={() => setExpanded(true)} />
          ) : null}
          {toasts.map((toast) => (
            <ToastItem key={toast.id} toast={toast} />
          ))}
        </div>
      </ToastPrimitive.Viewport>
    </ToastPrimitive.Portal>
  );
}
