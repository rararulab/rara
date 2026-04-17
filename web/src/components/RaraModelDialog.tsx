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

import { useEffect, useMemo, useRef, useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { api } from "@/api/client";
import type { ProviderInfo } from "@/api/types";

interface Props {
  open:              boolean;
  onClose:           () => void;
  onSelect:          (entry: ProviderInfo) => void;
  currentProvider?:  string | null;
}

/**
 * Rara-native LLM provider picker. Replaces pi-mono's `ModelSelector`,
 * whose catalog lives in pi-ai's hard-coded `MODELS` constant and
 * cannot address rara's custom OpenAI-compatible endpoints (`scnet`,
 * `stepfun`, `m3`, `local`, ...).
 *
 * Data source: `GET /api/v1/chat/providers` — a sanitised view of
 * `llm.providers.*` settings. Shows one entry per rara provider that
 * has a `default_model` configured. Sensitive `api_key` values never
 * cross the wire; the backend surfaces only a `has_api_key` boolean.
 */
export function RaraModelDialog({
  open,
  onClose,
  onSelect,
  currentProvider,
}: Props) {
  const [entries, setEntries] = useState<ProviderInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");
  const [selectedIdx, setSelectedIdx] = useState(0);

  // Cache the fetched list across open/close cycles — settings rarely
  // change and the dialog is opened per-click.
  const cacheRef = useRef<ProviderInfo[] | null>(null);

  useEffect(() => {
    if (!open) return;
    // Serve from cache on reopen; refetch only on first load.
    if (cacheRef.current) {
      setEntries(cacheRef.current);
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    api
      .get<ProviderInfo[]>("/api/v1/chat/providers", { signal: controller.signal })
      .then((list) => {
        cacheRef.current = list;
        setEntries(list);
      })
      .catch((e: unknown) => {
        if (controller.signal.aborted) return;
        console.warn("Failed to load provider catalog:", e);
        setEntries([]);
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [open]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter(
      (e) =>
        e.id.toLowerCase().includes(q) ||
        e.default_model.toLowerCase().includes(q),
    );
  }, [entries, query]);

  // Clamp the selection cursor whenever the filtered list shrinks/grows.
  useEffect(() => {
    setSelectedIdx((idx) =>
      filtered.length === 0 ? 0 : Math.min(idx, filtered.length - 1),
    );
  }, [filtered.length]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (filtered.length === 0) return;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIdx((i) => Math.min(i + 1, filtered.length - 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIdx((i) => Math.max(i - 1, 0));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const entry = filtered[selectedIdx];
      if (entry) onSelect(entry);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent className="max-w-lg" onKeyDown={handleKeyDown}>
        <DialogHeader>
          <DialogTitle>Select model</DialogTitle>
          <DialogDescription>
            Rara providers configured via{" "}
            <code className="text-[11px]">llm.providers.*</code> settings.
          </DialogDescription>
        </DialogHeader>

        <Input
          autoFocus
          placeholder="Filter by provider or model…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          className="mb-2"
        />

        <div className="max-h-[60vh] overflow-y-auto rounded border border-border">
          {loading ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              Loading…
            </div>
          ) : filtered.length === 0 ? (
            <div className="py-8 text-center text-sm text-muted-foreground">
              {entries.length === 0
                ? "No providers configured. Add one in /settings."
                : "No matches."}
            </div>
          ) : (
            filtered.map((entry, idx) => {
              const active = entry.id === currentProvider;
              const highlighted = idx === selectedIdx;
              return (
                <button
                  key={entry.id}
                  className={`flex w-full flex-col gap-0.5 border-b border-border/60 px-4 py-3 text-left transition-colors ${
                    highlighted ? "bg-secondary/80" : active ? "bg-secondary/50" : "hover:bg-secondary/40"
                  }`}
                  onMouseEnter={() => setSelectedIdx(idx)}
                  onClick={() => onSelect(entry)}
                >
                  <div className="flex items-center justify-between">
                    <span className="text-sm font-medium text-foreground">
                      {entry.id}
                    </span>
                    <div className="flex items-center gap-1">
                      {entry.enabled && (
                        <span className="rounded bg-green-500/15 px-1.5 py-0.5 text-[10px] font-medium text-green-600">
                          enabled
                        </span>
                      )}
                      {!entry.has_api_key && !entry.enabled && (
                        <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-medium text-amber-600">
                          no key
                        </span>
                      )}
                    </div>
                  </div>
                  <div className="font-mono text-xs text-muted-foreground">
                    {entry.default_model}
                  </div>
                  {entry.base_url && (
                    <div className="truncate text-[11px] text-muted-foreground/70">
                      {entry.base_url}
                    </div>
                  )}
                </button>
              );
            })
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
