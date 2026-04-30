// @ts-nocheck
/* Vendor stub: @craft-agent/shared/mentions */
/* eslint-disable @typescript-eslint/no-explicit-any */
export interface ParsedMentions {
  text: string;
  mentions: any[];
}

export function parseMentions(text: string): ParsedMentions {
  return { text, mentions: [] };
}

export function stripAllMentions(text: string): string {
  return text;
}

export function resolveSkillMentions(_text: string, _skills: any[]): any[] {
  return [];
}

export function resolveSourceMentions(_text: string, _sources: any[]): any[] {
  return [];
}
