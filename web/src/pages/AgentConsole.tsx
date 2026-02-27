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

import { useEffect, useRef, useState } from "react";
import { useSearchParams } from "react-router";
import {
  Activity,
  Bot,
  Briefcase,
  Clock,
  Ellipsis,
  Layers,
  Settings as SettingsIcon,
  Terminal,
} from "lucide-react";
import Chat from "@/pages/Chat";
import AgentStatus from "@/pages/AgentStatus";
import { AgentJobsPanel } from "@/pages/Scheduler";
import CodingTasks from "@/pages/CodingTasks";
import { cn } from "@/lib/utils";
import { useServerStatus } from "@/hooks/use-server-status";

const TOP_TABS = [
  { key: "chat", label: "Chat", icon: <Bot className="h-4 w-4" /> },
  { key: "ops", label: "Operations", icon: <Layers className="h-4 w-4" /> },
];

const OPS_TABS = [
  { key: "status", label: "Status", icon: <Activity className="h-4 w-4" /> },
  { key: "tasks", label: "Tasks", icon: <Terminal className="h-4 w-4" /> },
  {
    key: "scheduler",
    label: "Scheduler",
    icon: <Clock className="h-4 w-4" />,
  },
];

const OPS_UTILITY_ITEMS = [
  { href: "/jobs", label: "Jobs", icon: <Briefcase className="h-4 w-4" />, newTab: true },
  { href: "/settings", label: "Settings", icon: <SettingsIcon className="h-4 w-4" />, newTab: true },
];

function OperationsSidebarFooter() {
  const { isOnline, isChecking } = useServerStatus();
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);
  const closeTimerRef = useRef<number | null>(null);
  const statusText = isChecking
    ? "Connecting..."
    : isOnline
      ? "Server online"
      : "Server offline";

  const clearCloseTimer = () => {
    if (closeTimerRef.current !== null) {
      window.clearTimeout(closeTimerRef.current);
      closeTimerRef.current = null;
    }
  };

  const openMenu = () => {
    clearCloseTimer();
    setMenuOpen(true);
  };

  const scheduleCloseMenu = () => {
    clearCloseTimer();
    closeTimerRef.current = window.setTimeout(() => {
      setMenuOpen(false);
      closeTimerRef.current = null;
    }, 180);
  };

  useEffect(() => {
    if (!menuOpen) return;

    const onPointerDown = (event: MouseEvent) => {
      if (!menuRef.current?.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setMenuOpen(false);
      }
    };

    window.addEventListener("mousedown", onPointerDown);
    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("mousedown", onPointerDown);
      window.removeEventListener("keydown", onKeyDown);
    };
  }, [menuOpen]);

  useEffect(
    () => () => {
      clearCloseTimer();
    },
    [],
  );

  return (
    <div
      className="border-t border-border/70 bg-background/35 px-2 py-2"
    >
      <div className="flex items-end justify-between gap-2">
        <div
          className="relative"
          ref={menuRef}
          onMouseEnter={openMenu}
          onMouseLeave={scheduleCloseMenu}
          onBlur={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node | null)) {
              setMenuOpen(false);
            }
          }}
        >
          {menuOpen && (
            <div className="absolute bottom-full left-0 z-20 w-56 pb-2">
              <div className="rounded-xl border border-border/60 bg-background/95 p-1 shadow-lg shadow-black/5 backdrop-blur-md">
                <div className="space-y-1">
                  {OPS_UTILITY_ITEMS.map((item) => (
                    <a
                      key={item.href}
                      href={item.href}
                      target={item.newTab ? "_blank" : undefined}
                      rel={item.newTab ? "noreferrer" : undefined}
                      onClick={() => setMenuOpen(false)}
                      className="group flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium text-muted-foreground transition-all hover:bg-background/70 hover:text-foreground hover:ring-1 hover:ring-border/70"
                    >
                      {item.icon}
                      <span className="truncate">{item.label}</span>
                    </a>
                  ))}
                </div>
              </div>
            </div>
          )}
          <button
            type="button"
            title="More"
            onClick={() => setMenuOpen((v) => !v)}
            aria-expanded={menuOpen}
            aria-haspopup="menu"
            className="flex h-9 w-10 items-center justify-center rounded-lg text-muted-foreground transition-all hover:bg-background/70 hover:text-foreground"
          >
            <Ellipsis className="h-5 w-5" />
          </button>
        </div>
        <button
          type="button"
          title={statusText}
          aria-label={statusText}
          className="inline-flex h-9 w-9 items-center justify-center rounded-lg text-muted-foreground transition-all hover:bg-background/70"
        >
          <span
            className={cn(
              "h-2.5 w-2.5 shrink-0 rounded-full",
              isChecking && "bg-yellow-400 animate-pulse",
              isOnline && "bg-green-500",
              !isOnline && !isChecking && "bg-red-500",
            )}
          />
        </button>
      </div>
    </div>
  );
}

export default function AgentConsole() {
  const [searchParams, setSearchParams] = useSearchParams();
  const requestedTab = searchParams.get("tab") ?? "chat";
  const requestedOpsTab = searchParams.get("ops") ?? "status";
  const isLegacyOpsTab = OPS_TABS.some((t) => t.key === requestedTab);
  const topTab = requestedTab === "ops" || isLegacyOpsTab ? "ops" : "chat";
  const activeOpsTab = isLegacyOpsTab
    ? requestedTab
    : OPS_TABS.some((t) => t.key === requestedOpsTab)
      ? requestedOpsTab
      : "status";

  const setTopTab = (tab: string) => {
    if (tab === "chat") {
      setSearchParams({ tab: "chat", ops: activeOpsTab });
      return;
    }
    setSearchParams({ tab: "ops", ops: activeOpsTab });
  };

  const setOpsTab = (tab: string) => {
    setSearchParams({ tab: "ops", ops: tab });
  };

  return (
    <div className="flex h-full flex-col">
      {topTab === "chat" && (
        <div className="flex flex-1 min-h-0 flex-col">
          <Chat onOpenOperations={() => setTopTab("ops")} />
        </div>
      )}

      {topTab === "ops" && (
        <div className="relative flex flex-1 min-h-0 overflow-hidden">
          <aside className="absolute inset-y-3 left-3 z-20 flex w-64 shrink-0 flex-col overflow-hidden rounded-2xl border border-border/60 bg-background/92 shadow-xl shadow-black/5 backdrop-blur-md">
            <div className="border-b border-border/70 bg-background/40 px-3 py-2">
              <div className="grid w-full grid-cols-2 rounded-xl border border-border/70 bg-background/70 p-1">
                {TOP_TABS.map((tab) => {
                  const active = topTab === tab.key;
                  return (
                    <button
                      key={tab.key}
                      type="button"
                      onClick={() => setTopTab(tab.key)}
                      className={cn(
                        "rounded-lg px-2.5 py-1 text-xs transition-all",
                        active
                          ? "bg-primary/10 text-foreground ring-1 ring-primary/15"
                          : "text-muted-foreground hover:bg-background/80 hover:text-foreground",
                      )}
                    >
                      {tab.label}
                    </button>
                  );
                })}
              </div>
            </div>

            <nav className="min-h-0 flex-1 space-y-0.5 overflow-y-auto p-2">
              {OPS_TABS.map((tab) => {
                const active = activeOpsTab === tab.key;
                return (
                  <button
                    key={tab.key}
                    type="button"
                    onClick={() => setOpsTab(tab.key)}
                    className={cn(
                      "flex w-full items-center gap-3 rounded-xl px-2.5 py-2 text-left text-sm transition-all",
                      active
                        ? "bg-primary/10 text-foreground shadow-sm ring-1 ring-primary/15"
                        : "text-muted-foreground hover:bg-background/70 hover:text-foreground hover:ring-1 hover:ring-border/70",
                    )}
                  >
                    <span className={cn("shrink-0", active ? "text-primary" : "")}>
                      {tab.icon}
                    </span>
                    <span className="truncate font-medium">{tab.label}</span>
                  </button>
                );
              })}
            </nav>
            <OperationsSidebarFooter />
          </aside>

          <div className="flex h-full min-w-0 flex-1 p-2 transition-[padding] duration-200 md:p-3 md:pl-[17.75rem]">
            <div className="app-surface flex min-w-0 flex-1 overflow-auto rounded-2xl border border-border/60 shadow-sm">
              {activeOpsTab === "status" && <AgentStatus />}
              {activeOpsTab === "tasks" && (
                <div className="w-full p-6">
                  <CodingTasks />
                </div>
              )}
              {activeOpsTab === "scheduler" && (
                <div className="w-full p-6">
                  <AgentJobsPanel />
                </div>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
