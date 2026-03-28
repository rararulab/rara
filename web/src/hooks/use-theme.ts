/*
 * Copyright 2025 Rararulab
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *      http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";

export type Theme = "light" | "dark" | "system";

const STORAGE_KEY = "rara-theme";

/** Read persisted theme preference, defaulting to "system". */
function getStoredTheme(): Theme {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === "light" || raw === "dark" || raw === "system") return raw;
  } catch {
    // SSR or storage unavailable
  }
  return "system";
}

/** Resolve the effective dark/light state from a Theme value. */
function resolveIsDark(theme: Theme): boolean {
  if (theme === "system") {
    return window.matchMedia("(prefers-color-scheme: dark)").matches;
  }
  return theme === "dark";
}

/** Apply or remove the .dark class on the root element. */
function applyDarkClass(isDark: boolean) {
  document.documentElement.classList.toggle("dark", isDark);
}

// ---------------------------------------------------------------------------
// Tiny external store so all consumers share one reactive value.
// ---------------------------------------------------------------------------

let currentTheme: Theme = getStoredTheme();
const listeners = new Set<() => void>();

function subscribe(cb: () => void) {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

function getSnapshot(): Theme {
  return currentTheme;
}

function setThemeInternal(next: Theme) {
  currentTheme = next;
  try {
    window.localStorage.setItem(STORAGE_KEY, next);
  } catch {
    // quota exceeded
  }
  applyDarkClass(resolveIsDark(next));
  listeners.forEach((cb) => cb());
}

// Apply the initial dark class eagerly so the first render is correct.
applyDarkClass(resolveIsDark(currentTheme));

/** React hook providing theme state and controls. */
export function useTheme() {
  const theme = useSyncExternalStore(subscribe, getSnapshot, () => "system" as Theme);

  const isDark = useMemo(() => resolveIsDark(theme), [theme]);

  // Listen for OS theme changes when in "system" mode.
  useEffect(() => {
    if (theme !== "system") return;
    const mql = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => {
      applyDarkClass(mql.matches);
      // Notify listeners so isDark re-evaluates.
      listeners.forEach((cb) => cb());
    };
    mql.addEventListener("change", handler);
    return () => mql.removeEventListener("change", handler);
  }, [theme]);

  const setTheme = useCallback((t: Theme) => setThemeInternal(t), []);

  /** Cycle through light -> dark -> system. */
  const toggleTheme = useCallback(() => {
    const order: Theme[] = ["light", "dark", "system"];
    const idx = order.indexOf(currentTheme);
    setThemeInternal(order[(idx + 1) % order.length]);
  }, []);

  return { theme, isDark, setTheme, toggleTheme } as const;
}
