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

import { useCallback, useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/api/client";
import type { RuntimeSettingsPatch, RuntimeSettingsView } from "@/api/types";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  ChevronRight,
  Download,
  ExternalLink,
  Eye,
  EyeOff,
  MessageSquare,
  Search,
  SlidersHorizontal,
  Sparkles,
} from "lucide-react";

type SettingKey = "ai" | "telegram";
type ToastState = { kind: "success" | "error"; message: string } | null;
type OpenRouterModel = {
  id: string;
  name: string;
  contextLength: number | null;
};

function formatUpdatedAt(value: string | null): string {
  if (!value) return "Never";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString();
}

export default function Settings() {
  const queryClient = useQueryClient();
  const [aiModel, setAiModel] = useState("");
  const [aiApiKey, setAiApiKey] = useState("");
  const [showAiApiKey, setShowAiApiKey] = useState(false);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [models, setModels] = useState<OpenRouterModel[]>([]);
  const [telegramToken, setTelegramToken] = useState("");
  const [telegramChatId, setTelegramChatId] = useState("");
  const [selectedSetting, setSelectedSetting] = useState<SettingKey | null>(null);
  const [toast, setToast] = useState<ToastState>(null);

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.get<RuntimeSettingsView>("/api/v1/settings"),
  });

  useEffect(() => {
    if (!settingsQuery.data) return;
    setAiModel(settingsQuery.data.ai.model ?? "");
    setAiApiKey(settingsQuery.data.ai.openrouter_api_key ?? "");
    setTelegramChatId(
      settingsQuery.data.telegram.chat_id == null
        ? ""
        : String(settingsQuery.data.telegram.chat_id),
    );
  }, [settingsQuery.data]);

  const filteredModels = useMemo(() => {
    const q = modelSearch.trim().toLowerCase();
    const filtered = !q
      ? models
      : models.filter((m) => m.name.toLowerCase().includes(q) || m.id.toLowerCase().includes(q));
    return [...filtered].sort((a, b) => Number(b.id === aiModel) - Number(a.id === aiModel));
  }, [modelSearch, models]);

  const selectedModelForSave = useMemo(() => aiModel.trim(), [aiModel]);

  const patch = useMemo<RuntimeSettingsPatch | null>(() => {
    const current = settingsQuery.data;
    if (!current) return null;
    const next: RuntimeSettingsPatch = {};

    const aiPatch: NonNullable<RuntimeSettingsPatch["ai"]> = {};
    if (selectedModelForSave !== "" && selectedModelForSave !== (current.ai.model ?? "")) {
      aiPatch.model = selectedModelForSave;
    }
    if (aiApiKey.trim() !== "") {
      aiPatch.openrouter_api_key = aiApiKey.trim();
    }
    if (Object.keys(aiPatch).length > 0) {
      next.ai = aiPatch;
    }

    const telegramPatch: NonNullable<RuntimeSettingsPatch["telegram"]> = {};
    if (telegramToken.trim() !== "") {
      telegramPatch.bot_token = telegramToken.trim();
    }
    if (telegramChatId.trim() !== "") {
      const parsed = Number.parseInt(telegramChatId.trim(), 10);
      if (!Number.isFinite(parsed)) {
        return null;
      }
      if (parsed !== current.telegram.chat_id) {
        telegramPatch.chat_id = parsed;
      }
    }
    if (Object.keys(telegramPatch).length > 0) {
      next.telegram = telegramPatch;
    }

    return Object.keys(next).length > 0 ? next : null;
  }, [aiApiKey, selectedModelForSave, settingsQuery.data, telegramChatId, telegramToken]);

  const updateMutation = useMutation({
    mutationFn: (payload: RuntimeSettingsPatch) =>
      api.post<RuntimeSettingsView>("/api/v1/settings", payload),
    onSuccess: (updated) => {
      queryClient.setQueryData(["settings"], updated);
      setAiApiKey(updated.ai.openrouter_api_key ?? "");
      setShowAiApiKey(false);
      setTelegramToken("");
      setSelectedSetting(null);
      setToast({ kind: "success", message: "Settings updated successfully." });
    },
    onError: (e: unknown) => {
      const message = e instanceof Error ? e.message : "Failed to update settings";
      setToast({ kind: "error", message });
    },
  });

  const handleSave = () => {
    if (!settingsQuery.data) return;
    if (!patch) {
      setToast({ kind: "error", message: "No valid settings changes to save." });
      return;
    }
    updateMutation.mutate(patch);
  };

  const openSetting = (setting: SettingKey) => {
    setSelectedSetting(setting);
    if (setting === "ai") {
      setModelSearch("");
      setModelsError(null);
      setShowAiApiKey(false);
    }
  };

  const fetchModels = useCallback(async () => {
    const key = aiApiKey.trim() || settingsQuery.data?.ai.openrouter_api_key?.trim() || "";
    if (!key) {
      setModelsError("Please enter your OpenRouter API key first.");
      return;
    }
    setModelsLoading(true);
    setModelsError(null);
    try {
      const resp = await fetch("https://openrouter.ai/api/v1/models", {
        headers: {
          Authorization: `Bearer ${key}`,
        },
      });
      if (!resp.ok) {
        const text = await resp.text();
        throw new Error(text || "Failed to fetch models.");
      }
      const data = (await resp.json()) as {
        data?: Array<{
          id?: string;
          name?: string;
          context_length?: number;
          contextLength?: number;
        }>;
      };
      const loaded = (data.data ?? [])
        .map((m) => {
          const id = (m.id ?? "").trim();
          if (!id) return null;
          const name = (m.name ?? id).trim();
          const contextLength = m.context_length ?? m.contextLength ?? null;
          return {
            id,
            name,
            contextLength,
          } satisfies OpenRouterModel;
        })
        .filter((m): m is OpenRouterModel => Boolean(m));
      setModels(loaded);
      if (loaded.length > 0) {
        if (aiModel && loaded.some((m) => m.id === aiModel)) {
          setAiModel(aiModel);
        } else {
          setAiModel(loaded[0].id);
        }
      }
      setToast({ kind: "success", message: `Fetched ${loaded.length} models.` });
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : "Failed to fetch models";
      setModelsError(message);
    } finally {
      setModelsLoading(false);
    }
  }, [aiApiKey, aiModel, settingsQuery.data?.ai.openrouter_api_key]);

  const toggleModel = (id: string, checked: boolean) => {
    if (!checked) return;
    setAiModel(id);
  };

  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 3000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    if (selectedSetting !== "ai") return;
    if (!settingsQuery.data?.ai.openrouter_api_key) return;
    if (models.length > 0) return;
    if (modelsLoading) return;
    void fetchModels();
  }, [
    fetchModels,
    models.length,
    modelsLoading,
    selectedSetting,
    settingsQuery.data?.ai.openrouter_api_key,
  ]);

  if (settingsQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-56" />
        <Skeleton className="h-72 w-full" />
      </div>
    );
  }

  if (!settingsQuery.data) {
    return (
      <div className="space-y-4">
        <h1 className="text-2xl font-bold">Settings</h1>
        <p className="text-sm text-destructive">
          Failed to load settings. Please refresh and try again.
        </p>
      </div>
    );
  }

  const current = settingsQuery.data;
  const isDialogOpen = selectedSetting !== null;

  const dialogTitle =
    selectedSetting === "ai" ? "AI (OpenRouter)" : "Telegram Bot";

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Settings</h1>
        <p className="text-muted-foreground mt-2">
          Configure runtime credentials without restarting services.
        </p>
        <p className="text-xs text-muted-foreground mt-1">
          Last updated: {formatUpdatedAt(current.updated_at)}
        </p>
      </div>

      <div className="space-y-3">
        <button
          type="button"
          className="flex w-full items-center justify-between rounded-lg border p-4 text-left transition-colors hover:bg-accent"
          onClick={() => openSetting("ai")}
        >
          <div className="flex items-center gap-3">
            <Sparkles className="h-4 w-4 text-muted-foreground" />
            <div className="space-y-1">
              <p className="font-medium">AI (OpenRouter)</p>
              <p className="text-xs text-muted-foreground">
                Model: {current.ai.model ?? "Not set"} · Key:{" "}
                {current.ai.openrouter_api_key ? "Set" : "Not set"}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Badge variant={current.ai.configured ? "default" : "secondary"}>
              {current.ai.configured ? "Configured" : "Not configured"}
            </Badge>
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          </div>
        </button>

        <button
          type="button"
          className="flex w-full items-center justify-between rounded-lg border p-4 text-left transition-colors hover:bg-accent"
          onClick={() => openSetting("telegram")}
        >
          <div className="flex items-center gap-3">
            <MessageSquare className="h-4 w-4 text-muted-foreground" />
            <div className="space-y-1">
              <p className="font-medium">Telegram Bot</p>
              <p className="text-xs text-muted-foreground">
                Chat ID: {current.telegram.chat_id ?? "Not set"} · Token:{" "}
                {current.telegram.token_hint ?? "Not set"}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Badge variant={current.telegram.configured ? "default" : "secondary"}>
              {current.telegram.configured ? "Configured" : "Not configured"}
            </Badge>
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          </div>
        </button>
      </div>

      <Dialog open={isDialogOpen} onOpenChange={(open) => !open && setSelectedSetting(null)}>
        <DialogContent className="max-h-[85vh] overflow-y-auto p-0 sm:max-w-3xl">
          <DialogHeader className="border-b px-6 pb-4 pt-6">
            <DialogTitle>{dialogTitle}</DialogTitle>
            <DialogDescription>
              Review current values and update this setting.
            </DialogDescription>
          </DialogHeader>

          {selectedSetting === "ai" && (
            <div className="space-y-6 px-6 py-5">
              <div className="space-y-3 rounded-xl border bg-card p-4">
                <div className="flex items-center justify-between">
                  <Label htmlFor="ai-api-key" className="text-base font-semibold">
                    OpenRouter API Key
                  </Label>
                  <span className="text-xs text-muted-foreground">
                    {current.ai.openrouter_api_key ? "Current: Saved in settings" : "No key saved"}
                  </span>
                </div>
                <div className="flex items-center gap-2">
                  <Input
                    id="ai-api-key"
                    type={showAiApiKey ? "text" : "password"}
                    value={aiApiKey}
                    onChange={(e) => setAiApiKey(e.target.value)}
                    placeholder={current.ai.openrouter_api_key ?? "sk-or-v1-..."}
                    className="h-11"
                  />
                  <Button
                    type="button"
                    variant="outline"
                    size="icon"
                    className="h-11 w-11 shrink-0"
                    onClick={() => setShowAiApiKey((v) => !v)}
                  >
                    {showAiApiKey ? <EyeOff /> : <Eye />}
                  </Button>
                </div>
                <p className="text-sm text-muted-foreground">
                  Get your API key from{" "}
                  <a
                    href="https://openrouter.ai/keys"
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center gap-1 text-primary hover:underline"
                  >
                    OpenRouter Keys <ExternalLink className="h-3 w-3" />
                  </a>
                </p>
              </div>

              <div className="space-y-3 rounded-xl border bg-card p-4">
                <div className="flex items-center justify-between">
                  <Label htmlFor="models-search" className="text-xl font-semibold">
                    Models
                  </Label>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={fetchModels}
                    disabled={modelsLoading}
                    className="h-10 px-4"
                  >
                    <Download className="h-4 w-4" />
                    {modelsLoading ? "Fetching..." : "Fetch"}
                  </Button>
                </div>
                <div className="relative">
                  <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
                  <Input
                    id="models-search"
                    value={modelSearch}
                    onChange={(e) => setModelSearch(e.target.value)}
                    placeholder="Search models..."
                    className="h-11 pl-10"
                  />
                </div>
                <p className="text-sm text-muted-foreground">
                  Showing {filteredModels.length} models (enabled first)
                </p>
                <p className="text-sm">
                  Selected model:{" "}
                  <span className="font-medium">
                    {selectedModelForSave || current.ai.model || "Not set"}
                  </span>
                </p>
                {modelsError && (
                  <p className="text-sm text-destructive">{modelsError}</p>
                )}
                <div className="max-h-80 overflow-y-auto rounded-lg border bg-background">
                  {filteredModels.length === 0 && !selectedModelForSave && !current.ai.model && (
                    <div className="p-4 text-sm text-muted-foreground">
                      No models yet. Enter API key and click Fetch.
                    </div>
                  )}
                  {filteredModels.length === 0 && (selectedModelForSave || current.ai.model) && (
                    <div className="flex items-center justify-between gap-3 border-b px-3 py-2.5 last:border-b-0">
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-semibold">
                          Current Saved Model
                        </p>
                        <p className="truncate text-xs text-muted-foreground">
                          {selectedModelForSave || current.ai.model}
                        </p>
                      </div>
                    </div>
                  )}
                  {filteredModels.map((model) => (
                    <div
                      key={model.id}
                      className="flex items-center justify-between gap-3 border-b px-3 py-2.5 last:border-b-0"
                    >
                      <div className="min-w-0 flex-1">
                        <p className="truncate text-sm font-semibold">{model.name}</p>
                        <p className="truncate text-xs text-muted-foreground">
                          {model.id}
                          {model.contextLength ? ` · ${Math.round(model.contextLength / 1000)}K` : ""}
                        </p>
                      </div>
                      <div className="flex items-center gap-2">
                        <SlidersHorizontal className="h-3.5 w-3.5 text-muted-foreground" />
                        <Switch
                          checked={model.id === aiModel}
                          onCheckedChange={(checked) => toggleModel(model.id, checked)}
                        />
                      </div>
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}

          {selectedSetting === "telegram" && (
            <div className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="telegram-chat-id">Chat ID</Label>
                <Input
                  id="telegram-chat-id"
                  value={telegramChatId}
                  onChange={(e) => setTelegramChatId(e.target.value)}
                  placeholder="123456789"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="telegram-token">Bot Token</Label>
                <Input
                  id="telegram-token"
                  type="password"
                  value={telegramToken}
                  onChange={(e) => setTelegramToken(e.target.value)}
                  placeholder={current.telegram.token_hint ?? "123456:ABC..."}
                />
                <p className="text-xs text-muted-foreground">
                  Current token hint: {current.telegram.token_hint ?? "Not set"}
                </p>
              </div>
            </div>
          )}

          <DialogFooter className="border-t px-6 py-4">
            <Button
              variant="outline"
              onClick={() => setSelectedSetting(null)}
              disabled={updateMutation.isPending}
            >
              Cancel
            </Button>
            <Button onClick={handleSave} disabled={updateMutation.isPending}>
              {updateMutation.isPending ? "Saving..." : "Save Settings"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {toast && (
        <div className="fixed right-6 top-6 z-50">
          <div
            className={`rounded-md border px-4 py-3 text-sm shadow-lg ${
              toast.kind === "success"
                ? "border-green-200 bg-green-50 text-green-800"
                : "border-red-200 bg-red-50 text-red-800"
            }`}
          >
            {toast.message}
          </div>
        </div>
      )}
    </div>
  );
}
