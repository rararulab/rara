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

import { useEffect, useRef, useState } from "react";
import {
  Bot,
  Ellipsis,
  PanelLeftClose,
  PanelLeftOpen,
  Settings as SettingsIcon,
  Trash2,
} from "lucide-react";
import type { ChatSession } from "@/api/types";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import { useServerStatus } from "@/hooks/use-server-status";
import { formatTime } from "./utils";

// ---------------------------------------------------------------------------
// SessionSidebar components (left panel)
// ---------------------------------------------------------------------------

const chatUtilityItems = [
  { href: "/settings", icon: SettingsIcon, label: "Settings", newTab: true },
];

export function ConversationPanelToggleButton({
  collapsed,
  onToggle,
}: {
  collapsed: boolean;
  onToggle: () => void;
}) {
  return (
    <Button
      variant="ghost"
      size="icon"
      className="h-7 w-7 shrink-0 rounded-lg border border-transparent hover:border-border/70 hover:bg-background/70"
      onClick={onToggle}
      title={collapsed ? "Expand conversations" : "Collapse conversations"}
    >
      {collapsed ? (
        <PanelLeftOpen className="h-4 w-4" />
      ) : (
        <PanelLeftClose className="h-4 w-4" />
      )}
    </Button>
  );
}

function SessionSidebarUtilityBar({
  collapsed,
  onToggleCollapse,
}: {
  collapsed: boolean;
  onToggleCollapse: () => void;
}) {
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
      className={cn(
        "border-t border-border/70 bg-background/35",
        collapsed ? "p-1" : "px-2 py-2",
      )}
    >
      <div
        className={cn(
          "flex items-end",
          collapsed ? "justify-center" : "justify-between gap-2",
        )}
      >
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
                  {chatUtilityItems.map((item) => (
                    <a
                      key={item.href}
                      href={item.href}
                      target={item.newTab ? "_blank" : undefined}
                      rel={item.newTab ? "noreferrer" : undefined}
                      className="group flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-sm font-medium text-muted-foreground transition-all hover:bg-background/70 hover:text-foreground hover:ring-1 hover:ring-border/70"
                      onClick={() => setMenuOpen(false)}
                    >
                      <item.icon className="h-4 w-4 shrink-0" />
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
            className={cn(
              "flex h-9 items-center justify-center rounded-lg text-muted-foreground transition-all hover:bg-background/70 hover:text-foreground",
              collapsed ? "w-9" : "w-10",
            )}
            aria-expanded={menuOpen}
            aria-haspopup="menu"
          >
            <Ellipsis className="h-5 w-5" />
          </button>
        </div>

        {!collapsed ? (
          <div className="flex items-center gap-1">
            <ConversationPanelToggleButton
              collapsed={false}
              onToggle={onToggleCollapse}
            />
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
        ) : (
          <div className="flex items-center gap-1">
            <ConversationPanelToggleButton
              collapsed={false}
              onToggle={onToggleCollapse}
            />
            <button
              type="button"
              title={statusText}
              aria-label={statusText}
              className="inline-flex h-7 w-7 items-center justify-center rounded-md transition-all hover:bg-background/70"
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
        )}
      </div>
    </div>
  );
}

export function SessionList({
  sessions,
  activeKey,
  onSelect,
  onDelete,
  isLoading,
  collapsed,
  onToggleCollapse,
  onOpenOperations,
}: {
  sessions: ChatSession[];
  activeKey: string | null;
  onSelect: (key: string) => void;
  onDelete: (key: string) => void;
  isLoading: boolean;
  collapsed: boolean;
  onToggleCollapse: () => void;
  onOpenOperations: () => void;
}) {
  return (
    <div
      className={cn(
        "absolute inset-y-3 left-3 z-20 flex h-auto shrink-0 flex-col overflow-hidden rounded-2xl border border-border/60 bg-background/92 shadow-xl shadow-black/5 backdrop-blur-md transition-all duration-200",
        collapsed
          ? "pointer-events-none w-0 -translate-x-2 border-transparent opacity-0"
          : "w-64 opacity-100",
      )}
    >
      {!collapsed && (
        <>
          {/* Header */}
          <div className="border-b border-border/70 bg-background/40 px-3 py-2">
            <div className="grid w-full grid-cols-2 rounded-xl border border-border/70 bg-background/70 p-1">
              <button
                type="button"
                className="rounded-lg bg-primary/10 px-2.5 py-1 text-xs font-semibold text-foreground ring-1 ring-primary/15"
                aria-current="page"
              >
                Chat
              </button>
              <button
                type="button"
                onClick={onOpenOperations}
                className="rounded-lg px-2.5 py-1 text-xs font-medium text-muted-foreground transition-colors hover:bg-background/70 hover:text-foreground"
              >
                Operations
              </button>
            </div>
          </div>

          {/* Session list */}
          <div className="min-h-0 flex-1 overflow-y-auto">
            {isLoading && (
              <div className="space-y-2 p-2">
                {Array.from({ length: 4 }).map((_, i) => (
                  <Skeleton key={i} className="h-14 w-full" />
                ))}
              </div>
            )}
            {!isLoading && sessions.length === 0 && (
              <div className="p-4 text-center text-xs text-muted-foreground">
                No conversations yet.
                <br />
                Click &quot;New Chat&quot; to start.
              </div>
            )}
            {!isLoading && (
              <div className="space-y-0.5 p-2">
                {sessions.map((s) => (
                  <button
                    key={s.key}
                    type="button"
                    className={cn(
                      "group relative flex w-full items-center gap-2 rounded-xl px-2.5 py-2 text-left text-sm transition-all",
                      activeKey === s.key
                        ? "bg-primary/10 text-foreground ring-1 ring-primary/15"
                        : "text-muted-foreground hover:bg-background/70 hover:text-foreground hover:ring-1 hover:ring-border/60",
                    )}
                    onClick={() => onSelect(s.key)}
                  >
                    <Bot className="h-4 w-4 shrink-0" />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-sm font-medium">
                        {s.title ?? s.key}
                      </p>
                      {s.preview && (
                        <p className="truncate text-xs text-muted-foreground">
                          {s.preview}
                        </p>
                      )}
                    </div>
                    <span className="shrink-0 text-[10px] text-muted-foreground">
                      {formatTime(s.updated_at)}
                    </span>
                    <button
                      type="button"
                      className="absolute right-1 top-1 hidden rounded-md p-1 text-muted-foreground hover:bg-background/80 hover:text-destructive group-hover:block"
                      onClick={(e) => {
                        e.stopPropagation();
                        onDelete(s.key);
                      }}
                      title="Delete conversation"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </button>
                ))}
              </div>
            )}
          </div>

          <SessionSidebarUtilityBar
            collapsed={false}
            onToggleCollapse={onToggleCollapse}
          />
        </>
      )}
    </div>
  );
}
