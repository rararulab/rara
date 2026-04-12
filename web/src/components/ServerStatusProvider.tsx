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

import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { onlineManager } from "@tanstack/react-query";
import { ServerStatusContext } from "@/hooks/use-server-status";
import { resolveUrl } from "@/api/client";
const CHECK_INTERVAL_MS = 10_000;
const TIMEOUT_MS = 5_000;

export function ServerStatusProvider({ children }: { children: ReactNode }) {
  const [isOnline, setIsOnline] = useState(true);
  const [isChecking, setIsChecking] = useState(true);

  useEffect(() => {
    let cancelled = false;

    async function check() {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
      try {
        const res = await fetch(resolveUrl("/api/v1/health"), { signal: controller.signal });
        if (cancelled) return;
        const online = res.ok;
        setIsOnline(online);
        onlineManager.setOnline(online);
      } catch {
        if (cancelled) return;
        setIsOnline(false);
        onlineManager.setOnline(false);
      } finally {
        clearTimeout(timer);
        if (!cancelled) {
          setIsChecking(false);
        }
      }
    }

    check();
    const id = setInterval(check, CHECK_INTERVAL_MS);

    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  const value = useMemo(
    () => ({ isOnline, isChecking }),
    [isOnline, isChecking],
  );

  return (
    <ServerStatusContext.Provider value={value}>
      {children}
    </ServerStatusContext.Provider>
  );
}
