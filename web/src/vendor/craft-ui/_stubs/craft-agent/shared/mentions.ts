// @ts-nocheck
/* Vendor stub: @craft-agent/shared/mentions */
/* eslint-disable @typescript-eslint/no-explicit-any */
export interface ParsedMentions {
  text: string;
  mentions: any[];
  // RichTextInput reads `.skills/.sources/.files/.folders` from the parsed
  // result to decide whether to apply mention-aware line-height. The real
  // craft helper returns these as parallel arrays; the stub keeps them empty
  // so the consumer's `.length > 0` checks are safe instead of crashing on
  // undefined.
  skills: any[];
  sources: any[];
  files: any[];
  folders: any[];
}

export function parseMentions(_text: string, _skillSlugs?: any[], _sourceSlugs?: any[]): ParsedMentions {
  return { text: _text, mentions: [], skills: [], sources: [], files: [], folders: [] };
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
