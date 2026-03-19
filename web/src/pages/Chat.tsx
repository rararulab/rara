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

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  Bot,
  ChevronDown,
  Ellipsis,
  ImagePlus,
  Link2,
  Loader2,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Send,
  Settings as SettingsIcon,
  Star,
  Trash2,
  User,
  Wrench,
  X,
} from "lucide-react";
import { MessageTracePanel } from "@/components/MessageTracePanel";
import type { CascadeStreamState } from "@/hooks/use-cascade";
import { api } from "@/api/client";
import type {
  ChatMessageData,
  ChatModel,
  ChatSession,
} from "@/api/types";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { cn } from "@/lib/utils";
import {
  formatCompletedToolLine,
  formatLiveReasoning,
  formatToolCallSummary,
} from "@/lib/chat-progress";
import {
  buildOutboundChatContent,
  fileToImageBlock,
  imageBlockSrc,
  type ImageChatContentBlock,
} from "@/lib/chat-attachments";
import { useServerStatus } from "@/hooks/use-server-status";

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

function fetchSessions() {
  return api.get<ChatSession[]>("/api/v1/chat/sessions?limit=100&offset=0");
}

function fetchMessages(key: string) {
  return api.get<ChatMessageData[]>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages?limit=200`,
  );
}

function createSession(body: {
  key: string;
  title?: string;
  model?: string;
  system_prompt?: string;
}) {
  return api.post<ChatSession>("/api/v1/chat/sessions", body);
}

// ---------------------------------------------------------------------------
// WebSocket event types (matches Rust WebEvent enum)
// ---------------------------------------------------------------------------

type WebEvent =
  | { type: "message"; content: string }
  | { type: "typing" }
  | { type: "phase"; phase: string }
  | { type: "error"; message: string }
  | { type: "text_delta"; text: string }
  | { type: "reasoning_delta"; text: string }
  | { type: "tool_call_start"; name: string; id: string; arguments: Record<string, unknown> }
  | { type: "tool_call_end"; id: string; result_preview: string; success: boolean; error: string | null }
  | { type: "progress"; stage: string }
  | { type: "done" }
  | { type: "turn_rationale"; text: string }
  | { type: "turn_metrics"; duration_ms: number; iterations: number; tool_calls: number; model: string };

interface TurnMetrics {
  duration_ms: number;
  iterations: number;
  tool_calls: number;
  model: string;
}

interface ActiveToolCall {
  id: string;
  name: string;
  arguments: Record<string, unknown>;
}

interface CompletedTool {
  id: string;
  name: string;
  success: boolean;
  result_preview: string;
  error: string | null;
}

interface StreamState {
  isStreaming: boolean;
  text: string;
  reasoning: string;
  isThinking: boolean;
  activeTools: ActiveToolCall[];
  completedTools: CompletedTool[];
  turnRationale: string;
  error: string | null;
}

type PendingDraft = {
  text: string;
};

function deleteSession(key: string) {
  return api.del<void>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}`,
  );
}

function clearMessages(key: string) {
  return api.del<void>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/messages`,
  );
}

function fetchModels() {
  return api.get<ChatModel[]>("/api/v1/chat/models");
}

function setFavoriteModels(modelIds: string[]) {
  return api.put<string[]>("/api/v1/chat/models/favorites", {
    model_ids: modelIds,
  });
}

function updateSession(
  key: string,
  body: { title?: string; model?: string; system_prompt?: string },
) {
  return api.patch<ChatSession>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}`,
    body,
  );
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function generateKey(): string {
  return `chat-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

function extractTextContent(content: ChatMessageData["content"]): string {
  if (typeof content === "string") return content;
  return content
    .filter((b): b is { type: "text"; text: string } => b.type === "text")
    .map((b) => b.text)
    .join("");
}

function formatTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const now = new Date();
  if (d.toDateString() === now.toDateString()) {
    return d.toLocaleTimeString(undefined, {
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
}

// ---------------------------------------------------------------------------
// SessionList (left panel)
// ---------------------------------------------------------------------------

const chatUtilityItems = [
  { href: "/settings", icon: SettingsIcon, label: "Settings", newTab: true },
];

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

function ConversationPanelToggleButton({
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

function SessionList({
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

// ---------------------------------------------------------------------------
// MessageBubble
// ---------------------------------------------------------------------------

function ImageBlock({ src }: { src: string }) {
  const [failed, setFailed] = useState(false);

  if (failed) {
    return (
      <div className="flex h-32 w-48 items-center justify-center rounded-lg border border-dashed border-muted-foreground/30 bg-muted/30 text-xs text-muted-foreground">
        Image failed to load
      </div>
    );
  }

  return (
    <img
      src={src}
      alt=""
      className="max-h-64 max-w-xs rounded-lg object-contain"
      onError={() => setFailed(true)}
    />
  );
}

function MessageBubble({ msg, metrics, onClick }: { msg: ChatMessageData; metrics?: TurnMetrics | null; onClick?: () => void }) {
  const isUser = msg.role === "user";
  const isSystem = msg.role === "system";
  const isMultimodal = Array.isArray(msg.content);
  const text = extractTextContent(msg.content);

  if (isSystem) {
    return (
      <div className="mx-auto max-w-md rounded-full border border-border/70 bg-background/80 px-4 py-2 text-center text-xs text-muted-foreground italic shadow-sm">
        {text}
      </div>
    );
  }

  return (
    <div
      className={cn("group flex gap-3 cursor-pointer", isUser ? "flex-row-reverse" : "flex-row")}
      onClick={() => onClick?.()}
    >
      {/* Avatar */}
      <div
        className={cn(
          "mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl text-xs font-medium",
          isUser
            ? "bg-primary/90 text-primary-foreground"
            : "bg-background/60 text-muted-foreground",
        )}
      >
        {isUser ? <User className="h-4 w-4" /> : <Bot className="h-4 w-4" />}
      </div>

      {/* Content */}
      <div
        className={cn(
          isUser ? "max-w-[78%]" : "max-w-[min(78ch,calc(100%-4rem))] w-full",
          isUser
            ? "rounded-2xl bg-primary/90 px-4 py-2.5 text-primary-foreground"
            : "px-1 py-1 text-foreground",
        )}
      >
        {isMultimodal ? (
          <div className="space-y-2">
            {(msg.content as import("@/api/types").ChatContentBlock[]).map(
              (block, i) => {
                if (block.type === "text") {
                  return isUser ? (
                    <p key={i} className="whitespace-pre-wrap text-sm">
                      {block.text}
                    </p>
                  ) : (
                    <div
                      key={i}
                      className="prose prose-sm max-w-none text-foreground dark:prose-invert prose-p:text-foreground prose-li:text-foreground prose-strong:text-foreground prose-headings:text-foreground prose-code:text-foreground [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs"
                    >
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {block.text}
                      </ReactMarkdown>
                    </div>
                  );
                }
                if (block.type === "image_url" || block.type === "image_base64") {
                  return <ImageBlock key={i} src={imageBlockSrc(block)} />;
                }
                return null;
              },
            )}
          </div>
        ) : isUser ? (
          <p className="whitespace-pre-wrap text-sm">{text}</p>
        ) : (
          <div className="prose prose-sm max-w-none text-foreground dark:prose-invert prose-p:text-foreground prose-li:text-foreground prose-strong:text-foreground prose-headings:text-foreground prose-code:text-foreground [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
          </div>
        )}
        <p
          className={cn(
            "mt-1 text-[10px]",
            isUser ? "text-primary-foreground/70" : "text-muted-foreground",
          )}
        >
          {formatTime(msg.created_at)}
          {!isUser && metrics && (
            <span className="ml-2 opacity-0 group-hover:opacity-100 transition-opacity">
              {metrics.model.split("/").pop() ?? metrics.model} · {(metrics.duration_ms / 1000).toFixed(1)}s · {metrics.iterations} iter · {metrics.tool_calls} tools
            </span>
          )}
        </p>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ModelListPicker (searchable model list with favorites)
// ---------------------------------------------------------------------------

function formatContextLength(ctx: number): string {
  if (ctx >= 1_000_000) return `${(ctx / 1_000_000).toFixed(1)}M`;
  return `${(ctx / 1_000).toFixed(0)}K`;
}

function ModelListPicker({
  models,
  value,
  onValueChange,
  onToggleFavorite,
}: {
  models: ChatModel[];
  value: string;
  onValueChange: (value: string) => void;
  onToggleFavorite: (modelId: string, isFavorite: boolean) => void;
}) {
  const [search, setSearch] = useState("");

  const filtered = models.filter((m) => {
    if (!search.trim()) return true;
    const q = search.toLowerCase();
    return (
      m.id.toLowerCase().includes(q) || m.name.toLowerCase().includes(q)
    );
  });

  const favorites = filtered.filter((m) => m.is_favorite);
  const others = filtered.filter((m) => !m.is_favorite);

  return (
    <div className="overflow-hidden rounded-xl border border-input bg-card/70 shadow-sm">
      {/* Search */}
      <div className="flex items-center gap-2 border-b border-border/70 bg-background/40 px-3 py-2">
        <Search className="h-4 w-4 shrink-0 text-muted-foreground" />
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search models..."
          className="flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
        {search && (
          <button
            type="button"
            onClick={() => setSearch("")}
            className="text-muted-foreground hover:text-foreground"
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>

      {/* List */}
      <div className="max-h-60 overflow-y-auto">
        <div className="py-1">
          {favorites.length > 0 && (
            <>
              <p className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                Favorites
              </p>
              {favorites.map((m) => (
                <ModelRow
                  key={m.id}
                  model={m}
                  isSelected={m.id === value}
                  onSelect={() => onValueChange(m.id)}
                  onToggleFavorite={() =>
                    onToggleFavorite(m.id, m.is_favorite)
                  }
                />
              ))}
            </>
          )}
          {favorites.length > 0 && others.length > 0 && (
            <div className="my-1 border-t" />
          )}
          {others.length > 0 && (
            <>
              {favorites.length > 0 && (
                <p className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                  All Models
                </p>
              )}
              {others.map((m) => (
                <ModelRow
                  key={m.id}
                  model={m}
                  isSelected={m.id === value}
                  onSelect={() => onValueChange(m.id)}
                  onToggleFavorite={() =>
                    onToggleFavorite(m.id, m.is_favorite)
                  }
                />
              ))}
            </>
          )}
          {filtered.length === 0 && (
            <p className="px-3 py-4 text-center text-sm text-muted-foreground">
              No models found.
            </p>
          )}
        </div>
      </div>
    </div>
  );
}

function ModelRow({
  model,
  isSelected,
  onSelect,
  onToggleFavorite,
}: {
  model: ChatModel;
  isSelected: boolean;
  onSelect: () => void;
  onToggleFavorite: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex cursor-pointer items-center gap-2 px-3 py-2 text-sm transition-colors hover:bg-background/60",
        isSelected && "bg-primary/8 text-foreground",
      )}
    >
      <button
        type="button"
        onClick={(e) => {
          e.stopPropagation();
          onToggleFavorite();
        }}
        className="shrink-0"
        title={model.is_favorite ? "Remove from favorites" : "Add to favorites"}
      >
        <Star
          className={cn(
            "h-3.5 w-3.5 transition-colors",
            model.is_favorite
              ? "fill-yellow-400 text-yellow-400"
              : "text-muted-foreground/40 hover:text-yellow-400",
          )}
        />
      </button>
      <button
        type="button"
        className="flex min-w-0 flex-1 items-center gap-2 text-left"
        onClick={onSelect}
      >
        <span className="truncate font-medium">{model.name}</span>
        <span className="shrink-0 text-xs text-muted-foreground">
          {formatContextLength(model.context_length)}
        </span>
      </button>
      <span className="hidden shrink-0 truncate text-[10px] text-muted-foreground group-hover:inline max-w-[180px]">
        {model.id}
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// NewChatDialog
// ---------------------------------------------------------------------------

function NewChatDialog({
  open,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (title: string, model: string) => void;
}) {
  const queryClient = useQueryClient();
  const [title, setTitle] = useState(
    () =>
      `Chat ${new Date().toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}`,
  );
  const [selectedModel, setSelectedModel] = useState("");

  const modelsQuery = useQuery({
    queryKey: ["chat-models"],
    queryFn: fetchModels,
    staleTime: 5 * 60 * 1000,
  });

  const models = modelsQuery.data ?? [];

  const favoriteMutation = useMutation({
    mutationFn: setFavoriteModels,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["chat-models"] });
    },
  });

  const handleToggleFavorite = useCallback(
    (modelId: string, currentlyFavorite: boolean) => {
      const currentFavorites = models
        .filter((m) => m.is_favorite)
        .map((m) => m.id);
      const next = currentlyFavorite
        ? currentFavorites.filter((id) => id !== modelId)
        : [...currentFavorites, modelId];
      favoriteMutation.mutate(next);
    },
    [models, favoriteMutation],
  );

  // Set default model when models are loaded
  useEffect(() => {
    if (models.length > 0 && !selectedModel) {
      setSelectedModel(models[0].id);
    }
  }, [models, selectedModel]);

  // Reset form when dialog opens
  useEffect(() => {
    if (open) {
      setTitle(
        `Chat ${new Date().toLocaleString(undefined, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })}`,
      );
      if (models.length > 0) {
        setSelectedModel(models[0].id);
      }
    }
  }, [open, models]);

  const handleConfirm = useCallback(() => {
    if (!selectedModel) return;
    onConfirm(title.trim() || "New Chat", selectedModel);
    onOpenChange(false);
  }, [title, selectedModel, onConfirm, onOpenChange]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>New Conversation</DialogTitle>
          <DialogDescription>
            Choose a title and model for the new conversation.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <Label htmlFor="chat-title">Title</Label>
            <Input
              id="chat-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              placeholder="Conversation title"
            />
          </div>
          <div className="space-y-2">
            <Label>Model</Label>
            {modelsQuery.isLoading ? (
              <Skeleton className="h-9 w-full" />
            ) : (
              <ModelListPicker
                models={models}
                value={selectedModel}
                onValueChange={setSelectedModel}
                onToggleFavorite={handleToggleFavorite}
              />
            )}
          </div>
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button onClick={handleConfirm} disabled={!selectedModel}>
            Create
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// ChangeModelDialog
// ---------------------------------------------------------------------------

function ChangeModelDialog({
  open,
  onOpenChange,
  currentModel,
  onConfirm,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  currentModel: string;
  onConfirm: (model: string) => void;
}) {
  const queryClient = useQueryClient();
  const [selectedModel, setSelectedModel] = useState(currentModel);

  const modelsQuery = useQuery({
    queryKey: ["chat-models"],
    queryFn: fetchModels,
    staleTime: 5 * 60 * 1000,
  });

  const models = modelsQuery.data ?? [];

  const favoriteMutation = useMutation({
    mutationFn: setFavoriteModels,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["chat-models"] });
    },
  });

  const handleToggleFavorite = useCallback(
    (modelId: string, currentlyFavorite: boolean) => {
      const currentFavorites = models
        .filter((m) => m.is_favorite)
        .map((m) => m.id);
      const next = currentlyFavorite
        ? currentFavorites.filter((id) => id !== modelId)
        : [...currentFavorites, modelId];
      favoriteMutation.mutate(next);
    },
    [models, favoriteMutation],
  );

  useEffect(() => {
    if (open) {
      setSelectedModel(currentModel);
    }
  }, [open, currentModel]);

  const handleConfirm = useCallback(() => {
    if (!selectedModel) return;
    onConfirm(selectedModel);
    onOpenChange(false);
  }, [selectedModel, onConfirm, onOpenChange]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Change Model</DialogTitle>
          <DialogDescription>
            Select a different model for this conversation. Future messages will
            use the new model.
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-4 py-2">
          <div className="space-y-2">
            <Label>Model</Label>
            {modelsQuery.isLoading ? (
              <Skeleton className="h-9 w-full" />
            ) : (
              <ModelListPicker
                models={models}
                value={selectedModel}
                onValueChange={setSelectedModel}
                onToggleFavorite={handleToggleFavorite}
              />
            )}
          </div>
        </div>
        <DialogFooter>
          <Button
            variant="outline"
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            onClick={handleConfirm}
            disabled={!selectedModel || selectedModel === currentModel}
          >
            Switch Model
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------
// ChatThread (right panel)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// StreamingBubble — live assistant response during SSE streaming
// ---------------------------------------------------------------------------

function ActivityTree({ stream }: { stream: StreamState }) {
  const liveReasoning = formatLiveReasoning(stream.reasoning);

  if (
    stream.activeTools.length === 0 &&
    stream.completedTools.length === 0 &&
    !liveReasoning
  ) {
    return null;
  }
  return (
    <div className="mb-2 rounded-lg border border-border/50 bg-muted/30 px-3 py-2 text-xs font-mono text-muted-foreground space-y-1">
      {liveReasoning && (
        <div className="rounded-md border border-border/40 bg-background/60 px-2 py-1.5">
          <div className="mb-1 text-[10px] uppercase tracking-[0.2em] text-muted-foreground/60">
            Thinking
          </div>
          <div className="font-sans text-xs leading-5 text-foreground/85">
            {liveReasoning}
          </div>
        </div>
      )}
      {stream.turnRationale && (
        <div className="text-muted-foreground/70 text-[11px] leading-4" aria-label="LLM reasoning">
          💭 {stream.turnRationale}
        </div>
      )}
      {stream.completedTools.map((t) => (
        <div key={t.id}>
          <div className="flex items-start gap-1.5">
            <span className="break-words">
              {formatCompletedToolLine(t)}
            </span>
          </div>
        </div>
      ))}
      {stream.activeTools.map((t) => (
        <div key={t.id}>
          <div className="flex items-center gap-1.5">
            <span className="inline-block h-1.5 w-1.5 animate-spin rounded-full border border-blue-500 border-t-transparent" />
            <span className="break-words">
              {formatToolCallSummary(t.name, t.arguments)}
            </span>
          </div>
        </div>
      ))}
    </div>
  );
}

function StreamingBubble({ stream }: { stream: StreamState }) {
  const liveReasoning = formatLiveReasoning(stream.reasoning);

  return (
    <div className="flex gap-3">
      <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-background/60 text-muted-foreground">
        <Bot className="h-4 w-4" />
      </div>
      <div className="w-full max-w-[min(78ch,calc(100%-4rem))] px-1 py-1 text-foreground">
        {liveReasoning && (
          <div className="mb-3 rounded-2xl border border-border/50 bg-background/55 px-3 py-2">
            <div className="mb-1 text-[10px] uppercase tracking-[0.2em] text-muted-foreground/60">
              Thinking
            </div>
            <p className="text-sm leading-6 text-foreground/85">
              {liveReasoning}
            </p>
          </div>
        )}

        {/* Tool call indicators */}
        {stream.activeTools.length > 0 && (
          <div className="mb-2 space-y-1">
            {stream.activeTools.map((tool) => (
              <div
                key={tool.id}
                className="flex items-center gap-1.5 text-xs text-muted-foreground"
              >
                <Wrench className="h-3 w-3 animate-pulse" />
                <span className="font-mono">
                  {formatToolCallSummary(tool.name, tool.arguments)}
                </span>
              </div>
            ))}
          </div>
        )}

        {/* Thinking indicator */}
        {stream.isThinking &&
          !stream.text &&
          !liveReasoning &&
          stream.activeTools.length === 0 && (
          <div className="flex items-center gap-2">
            <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
            <span className="text-sm text-muted-foreground">Thinking...</span>
          </div>
          )}

        {/* Streaming text content */}
        {stream.text && (
          <div className="prose prose-sm max-w-none text-foreground dark:prose-invert prose-p:text-foreground prose-li:text-foreground prose-strong:text-foreground prose-headings:text-foreground prose-code:text-foreground [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>
              {stream.text}
            </ReactMarkdown>
          </div>
        )}

        {/* Error */}
        {stream.error && (
          <p className="text-sm text-destructive">{stream.error}</p>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ChatThread (right panel) — SSE streaming version
// ---------------------------------------------------------------------------

const INITIAL_STREAM_STATE: StreamState = {
  isStreaming: false,
  text: "",
  reasoning: "",
  isThinking: false,
  activeTools: [],
  completedTools: [],
  turnRationale: "",
  error: null,
};

function ChatThread({
  session,
  onClearMessages,
  panelCollapsed,
  onTogglePanel,
  initialDraft,
  onInitialDraftConsumed,
  onMessageClick,
  onStreamStateChange,
}: {
  session: ChatSession;
  onClearMessages: () => void;
  panelCollapsed: boolean;
  onTogglePanel: () => void;
  initialDraft?: PendingDraft | null;
  onInitialDraftConsumed?: () => void;
  onMessageClick?: (seq: number) => void;
  onStreamStateChange?: (isStreaming: boolean, state: CascadeStreamState) => void;
}) {
  const sessionKey = session.key;
  const queryClient = useQueryClient();
  const { isOnline } = useServerStatus();
  const [input, setInput] = useState("");
  const [attachments, setAttachments] = useState<ImageChatContentBlock[]>([]);
  const [imageInputVisible, setImageInputVisible] = useState(false);
  const [imageInputValue, setImageInputValue] = useState("");
  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const [stream, setStream] = useState<StreamState>(INITIAL_STREAM_STATE);
  const [latestMetrics, setLatestMetrics] = useState<TurnMetrics | null>(null);
  const wsRef = useRef<WebSocket | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Notify parent of stream state changes for cascade viewer
  useEffect(() => {
    onStreamStateChange?.(stream.isStreaming, {
      reasoning: stream.reasoning,
      activeTools: stream.activeTools,
      completedTools: stream.completedTools,
    });
  }, [stream.isStreaming, stream.reasoning, stream.activeTools, stream.completedTools, onStreamStateChange]);

  const messagesQuery = useQuery({
    queryKey: ["chat-messages", sessionKey],
    queryFn: () => fetchMessages(sessionKey),
    enabled: !!sessionKey,
  });

  const messages = messagesQuery.data ?? [];

  const handleAddImageUrl = useCallback(() => {
    const url = imageInputValue.trim();
    if (!url) return;
    setAttachments((prev) => [...prev, { type: "image_url", url }]);
    setImageInputValue("");
    setImageInputVisible(false);
  }, [imageInputValue]);

  const handleRemoveImageUrl = useCallback((index: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const handleFileSelection = useCallback(
    async (e: React.ChangeEvent<HTMLInputElement>) => {
      const inputEl = e.target;
      const files = Array.from(inputEl.files ?? []);
      inputEl.value = "";

      if (files.length === 0) return;

      try {
        const blocks = await Promise.all(files.map((file) => fileToImageBlock(file)));
        setAttachments((prev) => [...prev, ...blocks]);
      } catch {
        setStream((current) => ({
          ...current,
          error: "Failed to read image attachment",
        }));
      }
    },
    [],
  );

  const changeModelMutation = useMutation({
    mutationFn: (model: string) => updateSession(sessionKey, { model }),
    onSuccess: (_data, model) => {
      queryClient.setQueryData<ChatSession[]>(["chat-sessions"], (old) =>
        old?.map((s) => (s.key === sessionKey ? { ...s, model } : s)),
      );
    },
  });

  // WebSocket connection management
  // Uses a cleanedUp flag to handle React StrictMode double-mount gracefully,
  // and auto-reconnects on transient disconnects.
  useEffect(() => {
    if (!sessionKey) return;
    let cleanedUp = false;

    function connect() {
      if (cleanedUp) return;

      const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
      const baseUrl = import.meta.env.VITE_API_URL || "";
      const host = baseUrl ? new URL(baseUrl).host : window.location.host;
      const token = localStorage.getItem('access_token') ?? '';
      const url = `${protocol}//${host}/api/v1/kernel/chat/ws?session_key=${encodeURIComponent(sessionKey)}&token=${encodeURIComponent(token)}`;

      const ws = new WebSocket(url);
      wsRef.current = ws;

      ws.onopen = () => {
        if (cleanedUp) {
          ws.close();
          return;
        }
      };

      ws.onmessage = (e) => {
        if (cleanedUp) return;
        try {
          const event = JSON.parse(e.data) as WebEvent;
          switch (event.type) {
            case "text_delta":
              setStream((s) => ({ ...s, text: s.text + event.text }));
              break;
            case "reasoning_delta":
              setStream((s) => ({
                ...s,
                reasoning: s.reasoning + event.text,
              }));
              break;
            case "typing":
              setStream((s) => ({ ...s, isThinking: true }));
              break;
            case "turn_rationale":
              setStream((s) => ({
                ...s,
                turnRationale: event.text,
              }));
              break;
            case "tool_call_start":
              setStream((s) => ({
                ...s,
                activeTools: [
                  ...s.activeTools,
                  { id: event.id, name: event.name, arguments: event.arguments },
                ],
              }));
              break;
            case "tool_call_end": {
              setStream((s) => {
                const finished = s.activeTools.find((t) => t.id === event.id);
                return {
                  ...s,
                  activeTools: s.activeTools.filter((t) => t.id !== event.id),
                  completedTools: finished
                    ? [...s.completedTools, {
                        id: finished.id,
                        name: finished.name,
                        success: event.success,
                        result_preview: event.result_preview,
                        error: event.error,
                      }]
                    : s.completedTools,
                };
              });
              break;
            }
            case "progress":
              setStream((s) => ({
                ...s,
                isThinking: event.stage === "thinking",
              }));
              break;
            case "turn_metrics":
              setLatestMetrics({
                duration_ms: event.duration_ms,
                iterations: event.iterations,
                tool_calls: event.tool_calls,
                model: event.model,
              });
              break;
            case "done":
            case "message":
              setStream(INITIAL_STREAM_STATE);
              queryClient.invalidateQueries({
                queryKey: ["chat-messages", sessionKey],
              });
              queryClient.setQueryData<ChatSession[]>(
                ["chat-sessions"],
                (old) =>
                  old?.map((s) =>
                    s.key === sessionKey
                      ? {
                          ...s,
                          message_count: s.message_count + 2,
                          updated_at: new Date().toISOString(),
                        }
                      : s,
                  ),
              );
              break;
            case "error":
              setStream((s) => ({
                ...s,
                isStreaming: false,
                error: event.message,
              }));
              queryClient.invalidateQueries({
                queryKey: ["chat-messages", sessionKey],
              });
              break;
          }
        } catch {
          // Ignore non-JSON messages
        }
      };

      ws.onerror = () => {
        if (cleanedUp) return;
        setStream((s) => ({
          ...s,
          isStreaming: false,
          error: "WebSocket connection error",
        }));
      };

      ws.onclose = () => {
        // Only clear the ref if it still points to THIS WebSocket instance.
        // Prevents a stale onclose (from StrictMode's first mount) from
        // nullifying the ref set by the second mount's connect().
        if (wsRef.current === ws) {
          wsRef.current = null;
        }
        if (cleanedUp) return;
        // Auto-reconnect after delay
        setTimeout(() => connect(), 2000);
      };
    }

    connect();

    return () => {
      cleanedUp = true;
      if (wsRef.current) {
        wsRef.current.close();
        wsRef.current = null;
      }
    };
  }, [sessionKey, queryClient]);

  // WebSocket send
  const sendMessage = useCallback(
    (text: string, nextAttachments: ImageChatContentBlock[] = []) => {
      const trimmed = text.trim();
      if ((!trimmed && nextAttachments.length === 0) || stream.isStreaming || !isOnline) return;
      if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) return;

      setInput("");
      setAttachments([]);
      setImageInputVisible(false);
      setImageInputValue("");

      // Optimistically add user message to the cache
      const content = buildOutboundChatContent(trimmed, nextAttachments);
      const previous = queryClient.getQueryData<ChatMessageData[]>([
        "chat-messages",
        sessionKey,
      ]);
      const optimisticMsg: ChatMessageData = {
        seq: (previous?.length ?? 0) + 1,
        role: "user",
        content,
        created_at: new Date().toISOString(),
      };
      queryClient.setQueryData<ChatMessageData[]>(
        ["chat-messages", sessionKey],
        (old) => [...(old ?? []), optimisticMsg],
      );

      // Reset streaming state and send
      setStream({ ...INITIAL_STREAM_STATE, isStreaming: true });
      wsRef.current.send(
        typeof content === "string" ? content : JSON.stringify({ content }),
      );
    },
    [stream.isStreaming, isOnline, sessionKey, queryClient],
  );

  const handleSend = useCallback(() => {
    sendMessage(input, attachments);
  }, [attachments, input, sendMessage]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  const handleChangeModel = useCallback(
    (model: string) => {
      changeModelMutation.mutate(model);
    },
    [changeModelMutation],
  );

  // Auto-scroll to bottom (triggers on new messages or streaming text)
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length, stream.isStreaming, stream.text]);

  // Auto-focus textarea on session switch
  useEffect(() => {
    textareaRef.current?.focus();
  }, [sessionKey]);

  // Auto-resize textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [input]);

  useEffect(() => {
    if (!initialDraft?.text) return;
    sendMessage(initialDraft.text);
    onInitialDraftConsumed?.();
  }, [initialDraft, onInitialDraftConsumed, sendMessage]);

  // Visible messages (exclude system)
  const visibleMessages = messages.filter((m) => m.role !== "system");

  // Extract short model name for display (e.g. "openai/gpt-4o" -> "gpt-4o")
  const modelDisplay = session.model
    ? session.model.split("/").pop() ?? session.model
    : "default";

  const isBusy = stream.isStreaming;

  return (
    <div className="relative flex min-w-0 flex-1 flex-col">
      {/* Thread header */}
      <div className="absolute inset-x-4 top-3 z-10 md:inset-x-8">
        <div className="grid grid-cols-[auto_1fr_auto] items-start gap-3">
          <div className="flex min-w-0 items-center gap-2">
            {panelCollapsed && (
              <ConversationPanelToggleButton
                collapsed
                onToggle={onTogglePanel}
              />
            )}
          </div>

          <div className="min-w-0 text-center">
            <p className="truncate text-xs font-medium text-muted-foreground/90">
              {messages.length} message{messages.length !== 1 ? "s" : ""} · {session.title ?? sessionKey}
            </p>
          </div>

          <div className="flex items-center justify-end gap-2">
            <button
              type="button"
              onClick={() => setModelDialogOpen(true)}
              title={session.model ?? "Click to select a model"}
              className="shrink-0"
            >
              <Badge
                variant="secondary"
                className="cursor-pointer gap-1 border-0 bg-background/50 text-xs shadow-none backdrop-blur hover:bg-background/75"
              >
                {modelDisplay}
                <ChevronDown className="h-3 w-3" />
              </Badge>
            </button>
            <Button
              variant="ghost"
              size="sm"
              className="rounded-lg bg-background/35 text-muted-foreground backdrop-blur hover:bg-background/70 hover:text-destructive"
              onClick={onClearMessages}
              title="Clear messages"
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>
      </div>

      {/* Model change dialog */}
      <ChangeModelDialog
        open={modelDialogOpen}
        onOpenChange={setModelDialogOpen}
        currentModel={session.model ?? ""}
        onConfirm={handleChangeModel}
      />

      {/* Messages area */}
      <div className="flex-1 overflow-y-auto px-6 pb-40 pt-20 md:px-8 md:pb-44 md:pt-20">
        {messagesQuery.isLoading && (
          <div className="space-y-4">
            {Array.from({ length: 3 }).map((_, i) => (
              <div key={i} className="flex gap-3">
                <Skeleton className="h-8 w-8 rounded-full" />
                <Skeleton className="h-16 flex-1 rounded-xl" />
              </div>
            ))}
          </div>
        )}

        {!messagesQuery.isLoading && visibleMessages.length === 0 && !isBusy && (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
            <Bot className="h-12 w-12 opacity-20" />
            <p className="text-sm opacity-80">
              Start a conversation by typing a message below.
            </p>
          </div>
        )}

        {!messagesQuery.isLoading && (
          <div className="space-y-4">
            {visibleMessages.map((msg, i) => {
              const isLastAssistant =
                msg.role === "assistant" &&
                !visibleMessages.slice(i + 1).some((m) => m.role === "assistant");
              return (
                <MessageBubble
                  key={msg.seq}
                  msg={msg}
                  metrics={isLastAssistant ? latestMetrics : undefined}
                  onClick={() => onMessageClick?.(msg.seq)}
                />
              );
            })}

            {/* Activity tree (real-time tool call trace) */}
            {(stream.activeTools.length > 0 ||
              stream.completedTools.length > 0) && (
              <ActivityTree stream={stream} />
            )}

            {/* Live streaming assistant bubble */}
            {(stream.isStreaming || stream.text || stream.error) && (
              <StreamingBubble stream={stream} />
            )}

            {/* Non-streaming error (e.g. connection failure before stream starts) */}
            {stream.error && !stream.isStreaming && !stream.text && (
              <div className="mx-auto max-w-md rounded-md border border-destructive/30 bg-destructive/10 px-4 py-2 text-center text-sm text-destructive">
                {stream.error}
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>
        )}
      </div>

      {/* Input area */}
      <div className="pointer-events-none absolute inset-x-4 bottom-4 z-10 md:inset-x-8 md:bottom-6">
        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          multiple
          className="hidden"
          onChange={(e) => {
            void handleFileSelection(e);
          }}
        />

        {/* Attached image previews */}
        {attachments.length > 0 && (
          <div className="pointer-events-auto mb-2 flex flex-wrap gap-2">
            {attachments.map((block, i) => (
              <div
                key={i}
                className="group relative h-16 w-16 overflow-hidden rounded-xl border border-input bg-muted shadow-sm"
              >
                <img
                  src={imageBlockSrc(block)}
                  alt=""
                  className="h-full w-full object-cover"
                  onError={(e) => {
                    (e.target as HTMLImageElement).style.display = "none";
                  }}
                />
                <button
                  type="button"
                  className="absolute -right-1 -top-1 flex h-5 w-5 items-center justify-center rounded-full bg-destructive text-destructive-foreground opacity-0 shadow-sm transition-opacity group-hover:opacity-100"
                  onClick={() => handleRemoveImageUrl(i)}
                  title="Remove image"
                >
                  <X className="h-3 w-3" />
                </button>
              </div>
            ))}
          </div>
        )}

        {/* Image URL input */}
        {imageInputVisible && (
          <div className="pointer-events-auto mb-2 flex items-center gap-2 rounded-2xl border border-border/40 bg-background/70 p-2 shadow-lg backdrop-blur">
            <input
              type="url"
              value={imageInputValue}
              onChange={(e) => setImageInputValue(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  handleAddImageUrl();
                }
                if (e.key === "Escape") {
                  setImageInputVisible(false);
                  setImageInputValue("");
                }
              }}
              placeholder="Paste image URL and press Enter..."
              className="flex-1 rounded-lg border border-input bg-background px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              autoFocus
            />
            <Button
              variant="ghost"
              size="sm"
              className="rounded-lg"
              onClick={() => {
                setImageInputVisible(false);
                setImageInputValue("");
              }}
            >
              Cancel
            </Button>
          </div>
        )}

        <div className="pointer-events-auto flex items-end gap-2 rounded-2xl border border-border/40 bg-background/70 p-2 shadow-[0_10px_40px_rgba(15,23,42,0.12)] backdrop-blur-xl">
          <Button
            variant="ghost"
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl text-muted-foreground hover:bg-background/70 hover:text-foreground"
            onClick={() => fileInputRef.current?.click()}
            disabled={isBusy || !isOnline}
            title="Upload image"
          >
            <ImagePlus className="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl text-muted-foreground hover:bg-background/70 hover:text-foreground"
            onClick={() => setImageInputVisible((v) => !v)}
            disabled={isBusy || !isOnline}
            title="Attach image URL"
          >
            <Link2 className="h-4 w-4" />
          </Button>
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={isOnline ? "Type a message... (Enter to send, Shift+Enter for newline)" : "Server offline -- sending disabled"}
            rows={1}
            disabled={isBusy || !isOnline}
            autoFocus
            className="flex-1 resize-none appearance-none border-0 bg-transparent px-2 py-2.5 text-sm text-foreground shadow-none placeholder:text-muted-foreground focus:outline-none focus:ring-0 focus-visible:outline-none focus-visible:ring-0 disabled:cursor-not-allowed disabled:opacity-50"
          />
          <Button
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl shadow-sm"
            onClick={handleSend}
            disabled={(!input.trim() && attachments.length === 0) || isBusy || !isOnline}
            title={isOnline ? "Send message" : "Server offline"}
          >
            {isBusy ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Send className="h-4 w-4" />
            )}
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// EmptyState (when no session is selected)
// ---------------------------------------------------------------------------

function EmptyState({
  onSendFirstMessage,
  panelCollapsed,
  onTogglePanel,
}: {
  onSendFirstMessage: (text: string) => void;
  panelCollapsed: boolean;
  onTogglePanel: () => void;
}) {
  const { isOnline } = useServerStatus();
  const [input, setInput] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text || !isOnline) return;
    onSendFirstMessage(text);
    setInput("");
  }, [input, isOnline, onSendFirstMessage]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
      }
    },
    [handleSend],
  );

  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [input]);

  return (
    <div className="relative flex flex-1 flex-col">
      {panelCollapsed && (
        <div className="absolute left-4 top-4">
          <ConversationPanelToggleButton collapsed onToggle={onTogglePanel} />
        </div>
      )}

      <div className="flex-1" />

      <div className="pointer-events-none absolute inset-x-4 bottom-4 z-10 md:inset-x-8 md:bottom-6">
        <div className="pointer-events-auto flex items-end gap-2 rounded-2xl border border-border/40 bg-background/70 p-2 shadow-[0_10px_40px_rgba(15,23,42,0.12)] backdrop-blur-xl">
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={isOnline ? "Type a message... (Enter to send, Shift+Enter for newline)" : "Server offline -- sending disabled"}
            rows={1}
            disabled={!isOnline}
            autoFocus
            className="flex-1 resize-none appearance-none border-0 bg-transparent px-2 py-2.5 text-sm text-foreground shadow-none placeholder:text-muted-foreground focus:outline-none focus:ring-0 focus-visible:outline-none focus-visible:ring-0 disabled:cursor-not-allowed disabled:opacity-50"
          />
          <Button
            size="icon"
            className="h-10 w-10 shrink-0 rounded-xl shadow-sm"
            onClick={handleSend}
            disabled={!input.trim() || !isOnline}
            title={isOnline ? "Send message" : "Server offline"}
          >
            <Send className="h-4 w-4" />
          </Button>
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Chat (main page component)
// ---------------------------------------------------------------------------

export default function Chat({
  onOpenOperations,
}: {
  onOpenOperations?: () => void;
} = {}) {
  const queryClient = useQueryClient();
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [panelCollapsed, setPanelCollapsed] = useState(false);
  const [newChatDialogOpen, setNewChatDialogOpen] = useState(false);
  const [pendingDraft, setPendingDraft] = useState<PendingDraft | null>(null);
  const [cascadeSeq, setCascadeSeq] = useState<number | null>(null);
  const [threadStreaming, setThreadStreaming] = useState(false);
  const [threadStreamState, setThreadStreamState] = useState<CascadeStreamState>({
    reasoning: "",
    activeTools: [],
    completedTools: [],
  });

  const handleStreamStateChange = useCallback(
    (isStreaming: boolean, state: CascadeStreamState) => {
      setThreadStreaming(isStreaming);
      setThreadStreamState(state);
    },
    [],
  );

  // Clear cascade panel when switching sessions
  const switchSession = useCallback((key: string | null) => {
    setActiveKey(key);
    setCascadeSeq(null);
  }, []);

  const sessionsQuery = useQuery({
    queryKey: ["chat-sessions"],
    queryFn: fetchSessions,
  });

  // Hide internal agent sessions from the UI
  const sessions = (sessionsQuery.data ?? []).filter(
    (s) => !s.key.startsWith("agent:"),
  );

  const activeSession = activeKey
    ? sessions.find((s) => s.key === activeKey) ?? null
    : null;

  const createMutation = useMutation({
    mutationFn: createSession,
    onSuccess: (session) => {
      queryClient.setQueryData<ChatSession[]>(["chat-sessions"], (old) => {
        const next = old ?? [];
        if (next.some((s) => s.key === session.key)) return next;
        return [session, ...next];
      });
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      switchSession(session.key);
    },
  });

  const deleteMutation = useMutation({
    mutationFn: deleteSession,
    onSuccess: (_data, deletedKey) => {
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      queryClient.removeQueries({
        queryKey: ["chat-messages", deletedKey],
      });
      if (activeKey === deletedKey) {
        switchSession(null);
      }
    },
  });

  const clearMutation = useMutation({
    mutationFn: clearMessages,
    onSuccess: (_data, clearedKey) => {
      queryClient.invalidateQueries({
        queryKey: ["chat-messages", clearedKey],
      });
      queryClient.setQueryData<ChatSession[]>(["chat-sessions"], (old) =>
        old?.map((s) =>
          s.key === clearedKey
            ? { ...s, message_count: 0, preview: null }
            : s,
        ),
      );
    },
  });

  const handleCreateConfirm = useCallback(
    (title: string, model: string) => {
      const key = generateKey();
      createMutation.mutate({ key, title, model });
    },
    [createMutation],
  );

  const handleStartFromEmpty = useCallback(
    async (text: string) => {
      const trimmed = text.trim();
      if (!trimmed) return;
      const key = generateKey();
      setPendingDraft({ text: trimmed });
      try {
        await createMutation.mutateAsync({
          key,
          title: trimmed.slice(0, 80),
        });
      } catch {
        setPendingDraft(null);
      }
    },
    [createMutation],
  );

  const handleDelete = useCallback(
    (key: string) => {
      deleteMutation.mutate(key);
    },
    [deleteMutation],
  );

  const handleClearMessages = useCallback(() => {
    if (!activeKey) return;
    clearMutation.mutate(activeKey);
  }, [activeKey, clearMutation]);

  return (
    <div className="relative flex h-full overflow-hidden">
      {/* New chat dialog */}
      <NewChatDialog
        open={newChatDialogOpen}
        onOpenChange={setNewChatDialogOpen}
        onConfirm={handleCreateConfirm}
      />

      {/* Left panel: session list */}
      <SessionList
        sessions={sessions}
        activeKey={activeKey}
        onSelect={switchSession}
        onDelete={handleDelete}
        isLoading={sessionsQuery.isLoading}
        collapsed={panelCollapsed}
        onToggleCollapse={() => setPanelCollapsed((p) => !p)}
        onOpenOperations={() => onOpenOperations?.()}
      />

      <div
        className={cn(
          "flex h-full min-w-0 flex-1 p-2 md:p-3 transition-[padding] duration-200",
          panelCollapsed ? "" : "md:pl-[17.75rem]",
        )}
      >
        <div className="flex min-w-0 flex-1 overflow-hidden rounded-2xl bg-transparent">
          {/* Right panel: chat thread or empty state */}
          {activeSession ? (
            <div className="flex min-w-0 flex-1 overflow-hidden">
              <div className="min-w-0 flex-1">
                <ChatThread
                  key={activeKey}
                  session={activeSession}
                  onClearMessages={handleClearMessages}
                  panelCollapsed={panelCollapsed}
                  onTogglePanel={() => setPanelCollapsed((p) => !p)}
                  initialDraft={pendingDraft}
                  onInitialDraftConsumed={() => setPendingDraft(null)}
                  onMessageClick={(seq) => setCascadeSeq(seq)}
                  onStreamStateChange={handleStreamStateChange}
                />
              </div>
              {cascadeSeq !== null && activeKey && (
                <MessageTracePanel
                  sessionKey={activeKey}
                  messageSeq={cascadeSeq}
                  isStreaming={threadStreaming}
                  streamState={threadStreamState}
                  onClose={() => setCascadeSeq(null)}
                />
              )}
            </div>
          ) : (
            <EmptyState
              onSendFirstMessage={handleStartFromEmpty}
              panelCollapsed={panelCollapsed}
              onTogglePanel={() => setPanelCollapsed((p) => !p)}
            />
          )}
        </div>
      </div>
    </div>
  );
}
