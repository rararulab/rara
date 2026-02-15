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

import { useCallback, useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import {
  Bot,
  ChevronDown,
  ImagePlus,
  Loader2,
  MessageSquarePlus,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Send,
  Star,
  Trash2,
  User,
  X,
} from "lucide-react";
import { api } from "@/api/client";
import type {
  ChatContentBlock,
  ChatMessageData,
  ChatModel,
  ChatSession,
  SendMessageResponse,
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

function sendMessage(key: string, text: string, imageUrls?: string[]) {
  const body: { text: string; image_urls?: string[] } = { text };
  if (imageUrls && imageUrls.length > 0) {
    body.image_urls = imageUrls;
  }
  return api.post<SendMessageResponse>(
    `/api/v1/chat/sessions/${encodeURIComponent(key)}/send`,
    body,
  );
}

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

function SessionList({
  sessions,
  activeKey,
  onSelect,
  onCreate,
  onDelete,
  isLoading,
  collapsed,
  onToggleCollapse,
}: {
  sessions: ChatSession[];
  activeKey: string | null;
  onSelect: (key: string) => void;
  onCreate: () => void;
  onDelete: (key: string) => void;
  isLoading: boolean;
  collapsed: boolean;
  onToggleCollapse: () => void;
}) {
  return (
    <div
      className={cn(
        "flex flex-col border-r bg-card transition-all duration-200",
        collapsed ? "w-12" : "w-64",
      )}
    >
      {/* Header */}
      <div
        className={cn(
          "flex items-center border-b",
          collapsed ? "justify-center p-2" : "justify-between px-3 py-2",
        )}
      >
        {!collapsed && (
          <h2 className="text-sm font-semibold truncate">Conversations</h2>
        )}
        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0"
          onClick={onToggleCollapse}
          title={collapsed ? "Expand panel" : "Collapse panel"}
        >
          {collapsed ? (
            <PanelLeftOpen className="h-4 w-4" />
          ) : (
            <PanelLeftClose className="h-4 w-4" />
          )}
        </Button>
      </div>

      {/* New chat button */}
      <div className={cn("border-b", collapsed ? "p-1" : "p-2")}>
        <Button
          variant="outline"
          size={collapsed ? "icon" : "sm"}
          className={cn("shrink-0", collapsed ? "mx-auto h-8 w-8" : "w-full")}
          onClick={onCreate}
          title="New conversation"
        >
          <MessageSquarePlus className="h-4 w-4" />
          {!collapsed && <span className="ml-1.5">New Chat</span>}
        </Button>
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto">
        {isLoading && (
          <div className={cn("space-y-2", collapsed ? "p-1" : "p-2")}>
            {Array.from({ length: 4 }).map((_, i) => (
              <Skeleton
                key={i}
                className={cn(collapsed ? "h-8 w-8 mx-auto" : "h-14 w-full")}
              />
            ))}
          </div>
        )}
        {!isLoading && sessions.length === 0 && !collapsed && (
          <div className="p-4 text-center text-xs text-muted-foreground">
            No conversations yet.
            <br />
            Click &quot;New Chat&quot; to start.
          </div>
        )}
        {!isLoading && (
          <div className={cn("space-y-0.5", collapsed ? "p-1" : "p-2")}>
            {sessions.map((s) => (
              <button
                key={s.key}
                type="button"
                title={collapsed ? (s.title ?? s.key) : undefined}
                className={cn(
                  "group relative flex w-full items-center rounded-md text-left text-sm transition-colors",
                  collapsed
                    ? "justify-center p-2"
                    : "gap-2 px-2.5 py-2",
                  activeKey === s.key
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/50 hover:text-accent-foreground",
                )}
                onClick={() => onSelect(s.key)}
              >
                <Bot className="h-4 w-4 shrink-0" />
                {!collapsed && (
                  <>
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
                      className="absolute right-1 top-1 hidden rounded p-0.5 text-muted-foreground hover:text-destructive group-hover:block"
                      onClick={(e) => {
                        e.stopPropagation();
                        onDelete(s.key);
                      }}
                      title="Delete conversation"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </>
                )}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// MessageBubble
// ---------------------------------------------------------------------------

function ImageBlock({ url }: { url: string }) {
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
      src={url}
      alt=""
      className="max-h-64 max-w-xs rounded-lg object-contain"
      onError={() => setFailed(true)}
    />
  );
}

function MessageBubble({ msg }: { msg: ChatMessageData }) {
  const isUser = msg.role === "user";
  const isSystem = msg.role === "system";
  const isMultimodal = Array.isArray(msg.content);
  const text = extractTextContent(msg.content);

  if (isSystem) {
    return (
      <div className="mx-auto max-w-md rounded-md bg-muted/50 px-4 py-2 text-center text-xs text-muted-foreground italic">
        {text}
      </div>
    );
  }

  return (
    <div
      className={cn("flex gap-3", isUser ? "flex-row-reverse" : "flex-row")}
    >
      {/* Avatar */}
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-full text-xs font-medium",
          isUser
            ? "bg-primary text-primary-foreground"
            : "bg-muted text-muted-foreground",
        )}
      >
        {isUser ? <User className="h-4 w-4" /> : <Bot className="h-4 w-4" />}
      </div>

      {/* Content */}
      <div
        className={cn(
          "max-w-[75%] rounded-xl px-4 py-2.5",
          isUser
            ? "bg-primary text-primary-foreground"
            : "bg-muted text-foreground",
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
                      className="prose prose-sm dark:prose-invert max-w-none [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs"
                    >
                      <ReactMarkdown remarkPlugins={[remarkGfm]}>
                        {block.text}
                      </ReactMarkdown>
                    </div>
                  );
                }
                if (block.type === "image_url") {
                  return <ImageBlock key={i} url={block.url} />;
                }
                return null;
              },
            )}
          </div>
        ) : isUser ? (
          <p className="whitespace-pre-wrap text-sm">{text}</p>
        ) : (
          <div className="prose prose-sm dark:prose-invert max-w-none [&_pre]:overflow-x-auto [&_pre]:rounded-md [&_pre]:bg-background/50 [&_pre]:p-3 [&_code]:rounded [&_code]:bg-background/50 [&_code]:px-1 [&_code]:py-0.5 [&_code]:text-xs">
            <ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown>
          </div>
        )}
        <p
          className={cn(
            "mt-1 text-[10px]",
            isUser ? "text-primary-foreground/60" : "text-muted-foreground",
          )}
        >
          {formatTime(msg.created_at)}
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
    <div className="flex flex-col rounded-md border border-input">
      {/* Search */}
      <div className="flex items-center gap-2 border-b px-3 py-2">
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
        "group flex cursor-pointer items-center gap-2 px-3 py-1.5 text-sm transition-colors hover:bg-accent/50",
        isSelected && "bg-accent text-accent-foreground",
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

function ChatThread({
  session,
  onClearMessages,
}: {
  session: ChatSession;
  onClearMessages: () => void;
}) {
  const sessionKey = session.key;
  const queryClient = useQueryClient();
  const { isOnline } = useServerStatus();
  const [input, setInput] = useState("");
  const [imageUrls, setImageUrls] = useState<string[]>([]);
  const [imageInputVisible, setImageInputVisible] = useState(false);
  const [imageInputValue, setImageInputValue] = useState("");
  const [modelDialogOpen, setModelDialogOpen] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const messagesQuery = useQuery({
    queryKey: ["chat-messages", sessionKey],
    queryFn: () => fetchMessages(sessionKey),
    enabled: !!sessionKey,
  });

  const messages = messagesQuery.data ?? [];

  const sendMutation = useMutation({
    mutationFn: (vars: { text: string; imageUrls?: string[] }) =>
      sendMessage(sessionKey, vars.text, vars.imageUrls),
    onMutate: async (vars) => {
      // Cancel in-flight fetches so they don't overwrite optimistic update
      await queryClient.cancelQueries({
        queryKey: ["chat-messages", sessionKey],
      });

      const previous = queryClient.getQueryData<ChatMessageData[]>([
        "chat-messages",
        sessionKey,
      ]);

      // Build optimistic user message
      const content: ChatContentBlock[] | string = vars.imageUrls?.length
        ? [
            { type: "text" as const, text: vars.text },
            ...vars.imageUrls.map((url) => ({
              type: "image_url" as const,
              url,
            })),
          ]
        : vars.text;

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

      return { previous };
    },
    onError: (_err, _vars, context) => {
      // Roll back to previous messages on error
      if (context?.previous) {
        queryClient.setQueryData(
          ["chat-messages", sessionKey],
          context.previous,
        );
      }
    },
    onSettled: () => {
      // Always refetch to get the real server data (including assistant reply)
      queryClient.invalidateQueries({
        queryKey: ["chat-messages", sessionKey],
      });
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
    },
  });

  const handleAddImageUrl = useCallback(() => {
    const url = imageInputValue.trim();
    if (!url) return;
    setImageUrls((prev) => [...prev, url]);
    setImageInputValue("");
    setImageInputVisible(false);
  }, [imageInputValue]);

  const handleRemoveImageUrl = useCallback((index: number) => {
    setImageUrls((prev) => prev.filter((_, i) => i !== index));
  }, []);

  const changeModelMutation = useMutation({
    mutationFn: (model: string) => updateSession(sessionKey, { model }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
    },
  });

  const handleSend = useCallback(() => {
    const text = input.trim();
    if (!text || sendMutation.isPending || !isOnline) return;
    const urls = imageUrls.length > 0 ? [...imageUrls] : undefined;
    setInput("");
    setImageUrls([]);
    setImageInputVisible(false);
    setImageInputValue("");
    sendMutation.mutate({ text, imageUrls: urls });
  }, [input, imageUrls, sendMutation, isOnline]);

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

  // Auto-scroll to bottom
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length, sendMutation.isPending]);

  // Auto-resize textarea
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 160)}px`;
  }, [input]);

  // Visible messages (exclude system)
  const visibleMessages = messages.filter((m) => m.role !== "system");

  // Extract short model name for display (e.g. "openai/gpt-4o" -> "gpt-4o")
  const modelDisplay = session.model
    ? session.model.split("/").pop() ?? session.model
    : "default";

  return (
    <div className="flex flex-1 flex-col">
      {/* Thread header */}
      <div className="flex items-center justify-between border-b px-4 py-2.5">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <p className="truncate text-sm font-semibold">
              {session.title ?? sessionKey}
            </p>
            <button
              type="button"
              onClick={() => setModelDialogOpen(true)}
              title={session.model ?? "Click to select a model"}
              className="shrink-0"
            >
              <Badge
                variant="secondary"
                className="cursor-pointer gap-1 hover:bg-secondary/60"
              >
                {modelDisplay}
                <ChevronDown className="h-3 w-3" />
              </Badge>
            </button>
          </div>
          <p className="text-xs text-muted-foreground">
            {messages.length} message{messages.length !== 1 ? "s" : ""}
          </p>
        </div>
        <Button
          variant="ghost"
          size="sm"
          className="text-muted-foreground hover:text-destructive"
          onClick={onClearMessages}
          title="Clear messages"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </Button>
      </div>

      {/* Model change dialog */}
      <ChangeModelDialog
        open={modelDialogOpen}
        onOpenChange={setModelDialogOpen}
        currentModel={session.model ?? ""}
        onConfirm={handleChangeModel}
      />

      {/* Messages area */}
      <div className="flex-1 overflow-y-auto px-4 py-4">
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

        {!messagesQuery.isLoading && visibleMessages.length === 0 && (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
            <Bot className="h-12 w-12 opacity-30" />
            <p className="text-sm">
              Start a conversation by typing a message below.
            </p>
          </div>
        )}

        {!messagesQuery.isLoading && (
          <div className="space-y-4">
            {visibleMessages.map((msg) => (
              <MessageBubble key={msg.seq} msg={msg} />
            ))}

            {/* Pending assistant response indicator */}
            {sendMutation.isPending && (
              <div className="flex gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted text-muted-foreground">
                  <Bot className="h-4 w-4" />
                </div>
                <div className="flex items-center gap-2 rounded-xl bg-muted px-4 py-2.5">
                  <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" />
                  <span className="text-sm text-muted-foreground">
                    Thinking...
                  </span>
                </div>
              </div>
            )}

            {/* Error display */}
            {sendMutation.isError && (
              <div className="mx-auto max-w-md rounded-md border border-destructive/30 bg-destructive/10 px-4 py-2 text-center text-sm text-destructive">
                {sendMutation.error instanceof Error
                  ? sendMutation.error.message
                  : "Failed to send message. Please try again."}
              </div>
            )}

            <div ref={messagesEndRef} />
          </div>
        )}
      </div>

      {/* Input area */}
      <div className="border-t bg-card px-4 py-3">
        {!isOnline && (
          <p className="mb-2 text-center text-xs text-destructive">
            Server is offline. Sending is disabled until the connection is restored.
          </p>
        )}

        {/* Attached image previews */}
        {imageUrls.length > 0 && (
          <div className="mb-2 flex flex-wrap gap-2">
            {imageUrls.map((url, i) => (
              <div
                key={i}
                className="group relative h-16 w-16 overflow-hidden rounded-lg border border-input bg-muted"
              >
                <img
                  src={url}
                  alt=""
                  className="h-full w-full object-cover"
                  onError={(e) => {
                    (e.target as HTMLImageElement).style.display = "none";
                  }}
                />
                <button
                  type="button"
                  className="absolute -right-1 -top-1 flex h-5 w-5 items-center justify-center rounded-full bg-destructive text-destructive-foreground opacity-0 transition-opacity group-hover:opacity-100"
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
          <div className="mb-2 flex items-center gap-2">
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
              className="flex-1 rounded-md border border-input bg-background px-3 py-1.5 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              autoFocus
            />
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                setImageInputVisible(false);
                setImageInputValue("");
              }}
            >
              Cancel
            </Button>
          </div>
        )}

        <div className="flex items-end gap-2">
          <Button
            variant="ghost"
            size="icon"
            className="h-10 w-10 shrink-0 text-muted-foreground hover:text-foreground"
            onClick={() => setImageInputVisible((v) => !v)}
            disabled={sendMutation.isPending || !isOnline}
            title="Attach image URL"
          >
            <ImagePlus className="h-4 w-4" />
          </Button>
          <textarea
            ref={textareaRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder={isOnline ? "Type a message... (Enter to send, Shift+Enter for newline)" : "Server offline -- sending disabled"}
            rows={1}
            disabled={sendMutation.isPending || !isOnline}
            className="flex-1 resize-none rounded-lg border border-input bg-background px-3 py-2.5 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50"
          />
          <Button
            size="icon"
            className="h-10 w-10 shrink-0"
            onClick={handleSend}
            disabled={!input.trim() || sendMutation.isPending || !isOnline}
            title={isOnline ? "Send message" : "Server offline"}
          >
            {sendMutation.isPending ? (
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

function EmptyState({ onCreate }: { onCreate: () => void }) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 text-muted-foreground">
      <Bot className="h-16 w-16 opacity-20" />
      <div className="text-center">
        <p className="text-lg font-medium text-foreground">
          Welcome to Chat
        </p>
        <p className="mt-1 text-sm">
          Select a conversation from the sidebar or start a new one.
        </p>
      </div>
      <Button onClick={onCreate}>
        <MessageSquarePlus className="h-4 w-4" />
        New Conversation
      </Button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Chat (main page component)
// ---------------------------------------------------------------------------

export default function Chat() {
  const queryClient = useQueryClient();
  const [activeKey, setActiveKey] = useState<string | null>(null);
  const [panelCollapsed, setPanelCollapsed] = useState(false);
  const [newChatDialogOpen, setNewChatDialogOpen] = useState(false);

  const sessionsQuery = useQuery({
    queryKey: ["chat-sessions"],
    queryFn: fetchSessions,
    refetchInterval: 10_000,
  });

  // Hide internal agent sessions (e.g. "agent:proactive") from the UI
  const sessions = (sessionsQuery.data ?? []).filter(
    (s) => !s.key.startsWith("agent:"),
  );

  const activeSession = activeKey
    ? sessions.find((s) => s.key === activeKey) ?? null
    : null;

  const createMutation = useMutation({
    mutationFn: createSession,
    onSuccess: (session) => {
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
      setActiveKey(session.key);
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
        setActiveKey(null);
      }
    },
  });

  const clearMutation = useMutation({
    mutationFn: clearMessages,
    onSuccess: (_data, clearedKey) => {
      queryClient.invalidateQueries({
        queryKey: ["chat-messages", clearedKey],
      });
      queryClient.invalidateQueries({ queryKey: ["chat-sessions"] });
    },
  });

  const handleCreate = useCallback(() => {
    setNewChatDialogOpen(true);
  }, []);

  const handleCreateConfirm = useCallback(
    (title: string, model: string) => {
      const key = generateKey();
      createMutation.mutate({ key, title, model });
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
    <div className="flex h-full">
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
        onSelect={setActiveKey}
        onCreate={handleCreate}
        onDelete={handleDelete}
        isLoading={sessionsQuery.isLoading}
        collapsed={panelCollapsed}
        onToggleCollapse={() => setPanelCollapsed((p) => !p)}
      />

      {/* Right panel: chat thread or empty state */}
      {activeSession ? (
        <ChatThread
          key={activeKey}
          session={activeSession}
          onClearMessages={handleClearMessages}
        />
      ) : (
        <EmptyState onCreate={handleCreate} />
      )}
    </div>
  );
}
