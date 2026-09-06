export const GIT_PANEL_DEFAULT_WIDTH = 380;
export const GIT_PANEL_MIN_WIDTH = 320;
export const GIT_PANEL_MAX_WIDTH = 800;
export const GIT_PANEL_HANDLE_WIDTH = 4;
const SESSION_MIN_WIDTH = 400;

export function clampGitPanelWidth(width: number): number {
  return Number.isFinite(width)
    ? Math.min(GIT_PANEL_MAX_WIDTH, Math.max(GIT_PANEL_MIN_WIDTH, width))
    : GIT_PANEL_DEFAULT_WIDTH;
}

export function gitPanelLayout(containerWidth: number, preferredWidth: number) {
  const available = Number.isFinite(containerWidth) ? Math.max(0, containerWidth) : 0;
  const overlay = available < GIT_PANEL_MIN_WIDTH + SESSION_MIN_WIDTH + GIT_PANEL_HANDLE_WIDTH;
  const maxWidth = Math.min(
    GIT_PANEL_MAX_WIDTH,
    Math.max(0, available - GIT_PANEL_HANDLE_WIDTH - (overlay ? 0 : SESSION_MIN_WIDTH)),
  );
  const minWidth = Math.min(GIT_PANEL_MIN_WIDTH, maxWidth);
  return {
    overlay,
    minWidth,
    maxWidth,
    width: Math.min(maxWidth, clampGitPanelWidth(preferredWidth)),
  };
}
