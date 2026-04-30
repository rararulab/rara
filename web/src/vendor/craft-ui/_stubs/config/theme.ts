// @ts-nocheck
/* Vendor stub: @config/theme */
export const DEFAULT_THEME = { name: 'default', cssVars: {} };
export const DEFAULT_SHIKI_THEME = { name: 'github-dark-default', isDark: true };

export type ThemeOverrides = any;
export type ThemeFile = any;
export type ShikiThemeConfig = any;

export function resolveTheme(_input?: any): any {
  return DEFAULT_THEME;
}

export function themeToCSS(_theme: any): string {
  return '';
}

export function getShikiTheme(_name?: string): any {
  return DEFAULT_SHIKI_THEME;
}

export const THEME = DEFAULT_THEME;
