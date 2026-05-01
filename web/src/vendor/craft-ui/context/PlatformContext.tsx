// @ts-nocheck
/* Vendor stub for the Electron-only PlatformContext. Rara's web app is
 * always non-Electron, so we return a constant. */
import { createContext, useContext, type ReactNode } from 'react';

export interface PlatformInfo {
  isElectron: boolean;
  isMac: boolean;
  isWindows: boolean;
  isLinux: boolean;
  isMobile: boolean;
}

const DEFAULT_PLATFORM: PlatformInfo = {
  isElectron: false,
  isMac: false,
  isWindows: false,
  isLinux: false,
  isMobile: false,
};

const PlatformContext = createContext<PlatformInfo>(DEFAULT_PLATFORM);

export function PlatformProvider({ children }: { children: ReactNode }) {
  return <PlatformContext.Provider value={DEFAULT_PLATFORM}>{children}</PlatformContext.Provider>;
}

export function usePlatform(): PlatformInfo {
  return useContext(PlatformContext);
}
