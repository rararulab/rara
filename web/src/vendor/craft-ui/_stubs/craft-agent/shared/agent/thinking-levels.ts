// @ts-nocheck
/* Vendor stub: @craft-agent/shared/agent/thinking-levels */
export type ThinkingLevel = 'off' | 'low' | 'medium' | 'high';

export const THINKING_LEVELS: ThinkingLevel[] = ['off', 'low', 'medium', 'high'];
export const DEFAULT_THINKING_LEVEL: ThinkingLevel = 'off';

export function getThinkingLevelLabel(level: ThinkingLevel): string {
  return level.charAt(0).toUpperCase() + level.slice(1);
}

export function getThinkingLevelNameKey(level: ThinkingLevel): string {
  return `thinking.${level}`;
}
