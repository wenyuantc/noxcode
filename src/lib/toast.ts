import { Toast } from "@base-ui/react/toast";
import type { ReactNode } from "react";

/** Toast 视觉与语义变体：success / info 自动关闭，error / loading 需手动关闭。 */
export type ToastVariant = "success" | "info" | "warning" | "error" | "loading";

/** 单一栈限量：同一时刻最多展示的 toast 数量，超出的按最旧优先隐藏。 */
export const TOAST_LIMIT = 3;

/**
 * 用户点开「还有 N 条通知」后的栈上限：默认仍是 `TOAST_LIMIT`，
 * 只有主动展开时才把 limit 放宽到足以容纳当前全部通知，
 * 让被折叠的持久 error 不会被静默丢掉。
 */
export const TOAST_LIMIT_EXPANDED = Number.MAX_SAFE_INTEGER;

/**
 * 自动关闭时长（ms）：
 * - success / info 约 3s；
 * - warning 更久；
 * - error / loading 为 0，表示不自动关闭，直到用户点击或关闭。
 */
export const TOAST_TIMEOUT_MS: Record<ToastVariant, number> = {
  success: 3000,
  info: 3000,
  warning: 8000,
  error: 0,
  loading: 0,
};

/** 组件侧发起 toast 的入参。 */
export interface ToastInput {
  title?: ReactNode;
  description?: ReactNode;
  variant?: ToastVariant;
  /** 复用同一 id 会原地更新已有 toast 并重置计时，避免重复操作堆叠多条。 */
  id?: string;
  /** 覆盖该变体的默认自动关闭时长（ms）；0 表示不自动关闭。 */
  timeout?: number;
}

/** 传给 base-ui toast manager 的完整选项。 */
export interface ToastAddOptions {
  id?: string;
  type: ToastVariant;
  title?: ReactNode;
  description?: ReactNode;
  timeout: number;
  priority: "low" | "high";
}

/**
 * 全局 toast manager：宿主组件 `ToastProvider` 通过 `toastManager` prop 接入，
 * 业务代码直接调用 `showToast` / `dismissToast` 等 API，不经过额外的全局 state。
 */
export const toastManager = Toast.createToastManager();

export function toastTimeoutFor(variant: ToastVariant): number {
  return TOAST_TIMEOUT_MS[variant];
}

/** error 使用 high 优先级，走 aria-live 紧急播报；其余为礼貌播报。 */
export function toastPriorityFor(variant: ToastVariant): "low" | "high" {
  return variant === "error" ? "high" : "low";
}

/** 统计被 limit 折叠的通知时只依赖这两个字段，避免测试依赖 base-ui 的完整 ToastObject。 */
export interface ToastStackEntry {
  limited?: boolean;
  transitionStatus?: "starting" | "ending";
}

/**
 * 仍展示在栈里、但被 `limit` 折叠隐藏的通知数量，
 * 供「还有 N 条通知」入口使用；正在退场的条目不算隐藏。
 */
export function limitedToastCount(toasts: ReadonlyArray<ToastStackEntry>): number {
  return toasts.reduce(
    (count, toast) => (toast.limited && toast.transitionStatus !== "ending" ? count + 1 : count),
    0,
  );
}

export function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function toastAddOptions(input: ToastInput): ToastAddOptions {
  const variant = input.variant ?? "info";
  return {
    id: input.id,
    type: variant,
    title: input.title,
    description: input.description,
    timeout: input.timeout ?? toastTimeoutFor(variant),
    priority: toastPriorityFor(variant),
  };
}

/** 展示一条 toast，返回其 id（可用于后续 `updateToast` / `dismissToast`）。 */
export function showToast(input: ToastInput): string {
  return toastManager.add(toastAddOptions(input));
}

/** 关闭指定 toast；省略 id 时关闭全部。 */
export function dismissToast(id?: string): void {
  toastManager.close(id);
}

/** `updateToast` 的稀疏补丁：只覆盖显式给出的字段，其余保持原值。 */
export interface ToastUpdate {
  title?: ReactNode;
  description?: ReactNode;
  variant?: ToastVariant;
  /** 覆盖变体默认时长；只改文案时无需传，保留原有计时。 */
  timeout?: number;
}

/** 传给 base-ui toast manager 的更新选项。 */
export interface ToastUpdateOptions {
  type?: ToastVariant;
  title?: ReactNode;
  description?: ReactNode;
  timeout?: number;
  priority?: "low" | "high";
}

/**
 * 把稀疏补丁转换为更新选项：未给出的字段不写入，
 * 变体变化时同步刷新优先级，并且（未显式指定 timeout 时）按新变体重置计时。
 */
export function toastUpdateOptions(patch: ToastUpdate): ToastUpdateOptions {
  const updates: ToastUpdateOptions = {};
  if (patch.title !== undefined) updates.title = patch.title;
  if (patch.description !== undefined) updates.description = patch.description;
  if (patch.variant !== undefined) {
    updates.type = patch.variant;
    updates.priority = toastPriorityFor(patch.variant);
  }
  if (patch.timeout !== undefined) {
    updates.timeout = patch.timeout;
  } else if (patch.variant !== undefined) {
    updates.timeout = toastTimeoutFor(patch.variant);
  }
  return updates;
}

/**
 * 原地更新指定 toast，用于 loading 完成后替换为 success / error：
 * 只传需要变化的字段即可，未给出的字段保持原值。
 */
export function updateToast(id: string, patch: ToastUpdate): void {
  toastManager.update(id, toastUpdateOptions(patch));
}

export interface ToastActionOptions {
  /** 成功提示文案；省略表示成功时静默。 */
  successMessage?: ReactNode;
  successTitle?: ReactNode;
  /** 复用 id，使同一操作的多次结果原地更新。 */
  id?: string;
}

/**
 * 执行异步操作并按统一策略汇报结果：成功时（可选）提示 success，
 * 失败时提示不自动关闭的 error toast，并把错误信息完整展示。
 */
export async function runToastAction(
  action: () => Promise<unknown>,
  options: ToastActionOptions = {},
): Promise<boolean> {
  try {
    await action();
    if (options.successMessage !== undefined || options.successTitle !== undefined) {
      showToast({
        id: options.id,
        variant: "success",
        title: options.successTitle,
        description: options.successMessage,
      });
    }
    return true;
  } catch (error) {
    showToast({
      id: options.id,
      variant: "error",
      description: errorMessage(error),
    });
    return false;
  }
}
