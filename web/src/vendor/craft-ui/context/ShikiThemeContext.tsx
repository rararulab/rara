// @ts-nocheck
/* Vendor stub: shiki theme context. Returns a static dark theme name. */
import { createContext, useContext, type ReactNode } from 'react';

export interface ShikiThemeInfo {
  themeName: string;
  isDark: boolean;
}

const DEFAULT_THEME: ShikiThemeInfo = {
  themeName: 'github-dark-default',
  isDark: true,
};

const ShikiThemeContext = createContext<ShikiThemeInfo>(DEFAULT_THEME);

export function ShikiThemeProvider({ children }: { children: ReactNode }) {
  return <ShikiThemeContext.Provider value={DEFAULT_THEME}>{children}</ShikiThemeContext.Provider>;
}

export function useShikiTheme(): ShikiThemeInfo {
  return useContext(ShikiThemeContext);
}
