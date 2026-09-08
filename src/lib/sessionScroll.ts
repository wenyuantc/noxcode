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

export interface ScrollAnchor {
  key: string;
  offset: number;
}

export interface AnchorItem {
  key: string;
  top: number;
  bottom: number;
}

/** First item whose bottom is still below the viewport top. */
export function captureScrollAnchor(
  viewportTop: number,
  items: readonly AnchorItem[],
): ScrollAnchor | null {
  const hit = items.find((item) => item.key.length > 0 && item.bottom > viewportTop);
  if (!hit) return null;
  return { key: hit.key, offset: hit.top - viewportTop };
}

/** Add this delta to scrollTop so `itemTop - viewportTop` returns `savedOffset`. */
export function scrollDeltaForAnchor(
  itemTop: number,
  viewportTop: number,
  savedOffset: number,
): number {
  return itemTop - viewportTop - savedOffset;
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
