/*
 * Copyright 2025 Crrow
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

import { useEffect, useSyncExternalStore, useCallback, useRef } from "react";
import { useLocalStorage } from "./use-local-storage";

export type Theme = "system" | "light" | "dark";

const MEDIA_QUERY = "(prefers-color-scheme: dark)";

function getSystemDark(): boolean {
  return window.matchMedia(MEDIA_QUERY).matches;
}

function subscribe(cb: () => void): () => void {
  const mql = window.matchMedia(MEDIA_QUERY);
  mql.addEventListener("change", cb);
  return () => mql.removeEventListener("change", cb);
}

function applyTheme(isDark: boolean) {
  document.documentElement.classList.toggle("dark", isDark);
}

function prefersReducedMotion(): boolean {
  return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

type ViewTransitionCapableDocument = Document & {
  startViewTransition?: (update: () => void) => { finished?: Promise<unknown> };
};

function applyThemeWithAnimation(isDark: boolean, animate: boolean) {
  const root = document.documentElement;
  const body = document.body;

  if (!animate || prefersReducedMotion()) {
    applyTheme(isDark);
    return;
  }

  root.classList.add("theme-animating");
  body.classList.add("theme-animating");

  const cleanup = () => {
    root.classList.remove("theme-animating");
    body.classList.remove("theme-animating");
  };

  const doc = document as ViewTransitionCapableDocument;
  if (doc.startViewTransition) {
    const transition = doc.startViewTransition(() => applyTheme(isDark));
    transition.finished?.finally(cleanup);
    window.setTimeout(cleanup, 400);
    return;
  }

  applyTheme(isDark);
  window.setTimeout(cleanup, 260);
}

export function useTheme() {
  const [theme, setTheme] = useLocalStorage<Theme>("theme", "system");
  const systemDark = useSyncExternalStore(subscribe, getSystemDark);
  const mountedRef = useRef(false);

  const isDark = theme === "dark" || (theme === "system" && systemDark);

  useEffect(() => {
    applyThemeWithAnimation(isDark, mountedRef.current);
    mountedRef.current = true;
  }, [isDark]);

  const cycleTheme = useCallback(() => {
    setTheme((prev) => {
      if (prev === "system") return "light";
      if (prev === "light") return "dark";
      return "system";
    });
  }, [setTheme]);

  return { theme, setTheme, isDark, cycleTheme } as const;
}
