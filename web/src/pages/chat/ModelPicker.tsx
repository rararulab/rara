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

import { useCallback, useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Search, Star, X } from "lucide-react";
import type { ChatModel } from "@/api/types";
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
import { fetchModels, setFavoriteModels } from "./api";
import { formatContextLength } from "./utils";

// ---------------------------------------------------------------------------
// ModelListPicker (searchable model list with favorites)
// ---------------------------------------------------------------------------

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

export function ModelListPicker({
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

// ---------------------------------------------------------------------------
// NewChatDialog
// ---------------------------------------------------------------------------

export function NewChatDialog({
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

export function ChangeModelDialog({
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
