// @ts-nocheck
/* Vendor stub: @craft-agent/core/utils path helpers. */
export function normalizePath(p: string | null | undefined): string {
  if (!p) return '';
  return p.replace(/\\/g, '/');
}

export function pathStartsWith(p: string, prefix: string): boolean {
  return normalizePath(p).startsWith(normalizePath(prefix));
}

export function stripPathPrefix(p: string, prefix: string): string {
  const np = normalizePath(p);
  const npre = normalizePath(prefix);
  return np.startsWith(npre) ? np.slice(npre.length) : np;
}
