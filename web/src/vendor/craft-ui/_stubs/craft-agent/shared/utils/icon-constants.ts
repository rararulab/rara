// @ts-nocheck
/* Vendor stub: @craft-agent/shared/utils/icon-constants */
export function isEmoji(value: string | null | undefined): boolean {
  if (!value) return false;
  // Coarse heuristic, fine for the vendor consumers.
  return /\p{Extended_Pictographic}/u.test(value);
}
