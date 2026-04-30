// @ts-nocheck
/* Vendor stub: @craft-agent/shared/labels */
/* eslint-disable @typescript-eslint/no-explicit-any */
export type LabelConfig = any;

export function flattenLabels(_labels: any): any[] {
  return [];
}

export function flattenLabelsWithParentPath(_labels: any): any[] {
  return [];
}

export function parseLabelEntry(entry: string): { key: string; value: string } {
  const [key = '', value = ''] = entry.split(':');
  return { key, value };
}

export function formatLabelEntry(key: string, value: string): string {
  return `${key}:${value}`;
}

export function formatDisplayValue(value: any): string {
  return String(value ?? '');
}
