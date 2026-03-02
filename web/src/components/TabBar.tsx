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

import { useLayoutEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

export interface Tab {
  key: string;
  label: string;
  icon: React.ReactNode;
}

interface TabBarProps {
  tabs: Tab[];
  activeTab: string;
  onTabChange: (key: string) => void;
}

interface IndicatorStyle {
  left: number;
  width: number;
  visible: boolean;
}

export function TabBar({ tabs, activeTab, onTabChange }: TabBarProps) {
  const panelRef = useRef<HTMLDivElement | null>(null);
  const buttonRefs = useRef<Record<string, HTMLButtonElement | null>>({});
  const [indicator, setIndicator] = useState<IndicatorStyle>({
    left: 0,
    width: 0,
    visible: false,
  });

  useLayoutEffect(() => {
    const updateIndicator = () => {
      const activeEl = buttonRefs.current[activeTab];
      const panelEl = panelRef.current;
      if (!activeEl || !panelEl) {
        setIndicator((prev) => ({ ...prev, visible: false }));
        return;
      }

      setIndicator({
        left: activeEl.offsetLeft,
        width: activeEl.offsetWidth,
        visible: true,
      });
    };

    updateIndicator();

    const panelEl = panelRef.current;
    const resizeObserver =
      typeof ResizeObserver !== "undefined"
        ? new ResizeObserver(() => updateIndicator())
        : null;

    if (panelEl && resizeObserver) resizeObserver.observe(panelEl);
    Object.values(buttonRefs.current).forEach((el) => {
      if (el && resizeObserver) resizeObserver.observe(el);
    });

    window.addEventListener("resize", updateIndicator);
    return () => {
      window.removeEventListener("resize", updateIndicator);
      resizeObserver?.disconnect();
    };
  }, [activeTab, tabs]);

  return (
    <div className="sticky top-0 z-10 overflow-x-auto px-2 py-2 md:px-3">
      <div
        ref={panelRef}
        className="data-panel relative inline-flex min-w-max items-center gap-1.5 p-1"
      >
        <div
          aria-hidden="true"
          className={cn(
            "pointer-events-none absolute top-1 bottom-1 rounded-xl bg-background shadow-sm ring-1 ring-border/70 transition-[left,width,opacity,transform] duration-250 ease-out",
            indicator.visible ? "opacity-100" : "opacity-0"
          )}
          style={{
            left: indicator.left,
            width: indicator.width,
            transform: `translateZ(0) scale(${indicator.visible ? 1 : 0.98})`,
          }}
        />
        {tabs.map((tab) => (
          <button
            key={tab.key}
            ref={(el) => {
              buttonRefs.current[tab.key] = el;
            }}
            type="button"
            aria-pressed={activeTab === tab.key}
            onClick={() => onTabChange(tab.key)}
            className={cn(
              "group relative z-10 flex shrink-0 items-center gap-2 rounded-xl px-3 py-2 text-sm font-medium transition-all",
              "focus-visible:ring-2 focus-visible:ring-ring/50",
              activeTab === tab.key
                ? "text-foreground"
                : "text-muted-foreground hover:-translate-y-0.5 hover:bg-background/70 hover:text-foreground hover:shadow-sm",
            )}
          >
            <span
              className={cn(
                "opacity-80 transition-transform",
                activeTab === tab.key
                  ? "opacity-100 text-primary"
                  : "group-hover:scale-105",
              )}
            >
              {tab.icon}
            </span>
            {tab.label}
          </button>
        ))}
      </div>
    </div>
  );
}
