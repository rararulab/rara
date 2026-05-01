// @ts-nocheck
/* Vendor stub: browser pane helpers used by ToolbarStatusSlot. */
export function getHostname(url: string | null | undefined): string {
  if (!url) return '';
  try {
    return new URL(url).hostname;
  } catch {
    return '';
  }
}

export function getThemeLuminance(_color: string): number {
  return 0.5;
}
