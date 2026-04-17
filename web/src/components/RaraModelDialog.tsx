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
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { settingsApi } from "@/api/client";

/**
 * A rara-native LLM provider entry assembled from `/api/v1/settings`.
 *
 * Rara's provider catalog lives entirely in flat KV settings under
 * `llm.providers.<id>.*`. Provider ids come straight from rara
 * (`openrouter`, `kimi`, `minimax`, `glm`, `scnet`, `stepfun`, ...) so
 * the backend's `DriverRegistry::resolve` can route directly when we
 * PATCH this back onto the session.
 */
export interface RaraProviderEntry {
  id:             string;
  default_model:  string;
  base_url?:      string;
  has_api_key:    boolean;
  enabled:        boolean;
}

interface Props {
  open:              boolean;
  onClose:           () => void;
  onSelect:          (entry: RaraProviderEntry) => void;
  currentProvider?:  string | null;
}

/**
 * Model picker backed by rara's own settings. Replaces pi-mono's
 * `ModelSelector`, whose catalog lives in pi-ai's hard-coded `MODELS`
 * constant and cannot address rara's custom OpenAI-compatible
 * endpoints (`scnet`, `stepfun`, `m3`, `local`, ...). Shows one entry
 * per rara provider that has a `default_model` configured.
 */
export function RaraModelDialog({
  open,
  onClose,
  onSelect,
  currentProvider,
}: Props) {
  const [entries, setEntries] = useState<RaraProviderEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [query, setQuery] = useState("");

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    settingsApi
      .list()
      .then((settings) => setEntries(parseProviderEntries(settings)))
      .catch((e) => {
        console.warn("Failed to load provider catalog:", e);
        setEntries([]);
      })
      .finally(() => setLoading(false));
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

  return (
    <Dialog open={open} onOpenChange={(next) => { if (!next) onClose(); }}>
      <DialogContent className="max-w-lg">
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
            filtered.map((entry) => {
              const active = entry.id === currentProvider;
              return (
                <button
                  key={entry.id}
                  className={`flex w-full flex-col gap-0.5 border-b border-border/60 px-4 py-3 text-left transition-colors hover:bg-secondary/60 ${
                    active ? "bg-secondary/80" : ""
                  }`}
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

/**
 * Extract provider entries from the flat settings map. Keep any provider
 * that has a non-empty `default_model`; surface enabled / api-key flags
 * so the UI can warn but still let the user pick.
 */
function parseProviderEntries(settings: Record<string, string>): RaraProviderEntry[] {
  // Group keys by provider id.
  const byId = new Map<string, Record<string, string>>();
  for (const [key, value] of Object.entries(settings)) {
    const m = /^llm\.providers\.([^.]+)\.([^.]+)$/.exec(key);
    if (!m) continue;
    const [, id, field] = m;
    let bucket = byId.get(id);
    if (!bucket) {
      bucket = {};
      byId.set(id, bucket);
    }
    bucket[field] = value;
  }

  const entries: RaraProviderEntry[] = [];
  for (const [id, fields] of byId.entries()) {
    const defaultModel = (fields["default_model"] ?? "").trim();
    if (!defaultModel) continue;
    entries.push({
      id,
      default_model: defaultModel,
      base_url:      (fields["base_url"] ?? "").trim() || undefined,
      has_api_key:   (fields["api_key"] ?? "").trim().length > 0,
      enabled:       fields["enabled"] === "true",
    });
  }

  // Enabled first, then providers with api_key, then the rest.
  entries.sort((a, b) => {
    const score = (e: RaraProviderEntry) =>
      (e.enabled ? 2 : 0) + (e.has_api_key ? 1 : 0);
    const diff = score(b) - score(a);
    return diff !== 0 ? diff : a.id.localeCompare(b.id);
  });

  return entries;
}
