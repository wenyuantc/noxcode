export const BOTTOM_THRESHOLD = 80;

export interface ScrollMetrics {
  scrollHeight: number;
  scrollTop: number;
  clientHeight: number;
}

/** `clientHeight <= 0` 视为未布局，不判离底。 */
export function isNearBottom(metrics: ScrollMetrics, threshold = BOTTOM_THRESHOLD): boolean {
  if (metrics.clientHeight <= 0) return true;
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= threshold;
}

export function pinAfterUserScroll({
  programmatic,
  clientHeight,
  nearBottom,
  previous,
}: {
  programmatic: boolean;
  clientHeight: number;
  nearBottom: boolean;
  previous: boolean;
}): boolean {
  if (clientHeight <= 0 || programmatic) return previous;
  return nearBottom;
}
