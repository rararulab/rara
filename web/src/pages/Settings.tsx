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
import type {
  PromptFileView,
  PromptListView,
  RuntimeSettingsPatch,
  RuntimeSettingsView,
} from "@/api/types";
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
  ArrowDown,
  ArrowUp,
  Bot,
  ChevronRight,
  Download,
  ExternalLink,
  Eye,
  EyeOff,
  MessageSquare,
  Search,
  Sparkles,
  X,
} from "lucide-react";
import { Textarea } from "@/components/ui/textarea";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

type SettingKey = "ai" | "agent" | "telegram";
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
  const [defaultModel, setDefaultModel] = useState("");
  const [jobModels, setJobModels] = useState<string[]>([]);   // ordered: [primary, fallback1, fallback2, ...]
  const [chatModels, setChatModels] = useState<string[]>([]); // ordered: [primary, fallback1, fallback2, ...]
  const [aiApiKey, setAiApiKey] = useState("");
  const [showAiApiKey, setShowAiApiKey] = useState(false);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [models, setModels] = useState<OpenRouterModel[]>([]);
  const [telegramToken, setTelegramToken] = useState("");
  const [telegramChatId, setTelegramChatId] = useState("");
  const [telegramAllowedGroupChatId, setTelegramAllowedGroupChatId] = useState("");
  const [selectedPromptName, setSelectedPromptName] = useState("");
  const [selectedPromptContent, setSelectedPromptContent] = useState("");
  const [promptDirty, setPromptDirty] = useState(false);
  const [selectedSetting, setSelectedSetting] = useState<SettingKey | null>(null);
  const [toast, setToast] = useState<ToastState>(null);

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.get<RuntimeSettingsView>("/api/v1/settings"),
  });

  const promptsQuery = useQuery({
    queryKey: ["settings-prompts"],
    queryFn: () => api.get<PromptListView>("/api/v1/settings/prompts"),
  });

  useEffect(() => {
    if (!settingsQuery.data) return;
    setDefaultModel(settingsQuery.data.ai.default_model ?? "");

    // Build ordered model lists: [primary, ...fallbacks]
    const jobPrimary = settingsQuery.data.ai.job_model;
    const jobFallbacks = settingsQuery.data.ai.job_model_fallbacks ?? [];
    setJobModels(jobPrimary ? [jobPrimary, ...jobFallbacks] : []);

    const chatPrimary = settingsQuery.data.ai.chat_model;
    const chatFallbacks = settingsQuery.data.ai.chat_model_fallbacks ?? [];
    setChatModels(chatPrimary ? [chatPrimary, ...chatFallbacks] : []);

    setAiApiKey(settingsQuery.data.ai.openrouter_api_key ?? "");
    setTelegramChatId(
      settingsQuery.data.telegram.chat_id == null
        ? ""
        : String(settingsQuery.data.telegram.chat_id),
    );
    setTelegramAllowedGroupChatId(
      settingsQuery.data.telegram.allowed_group_chat_id == null
        ? ""
        : String(settingsQuery.data.telegram.allowed_group_chat_id),
    );
  }, [settingsQuery.data]);

  useEffect(() => {
    const prompts = promptsQuery.data?.prompts ?? [];
    if (prompts.length === 0) return;

    if (!selectedPromptName || !prompts.some((p) => p.name === selectedPromptName)) {
      const preferred = prompts.find((p) => p.name === "agent/soul.md");
      const initial = preferred ?? prompts[0];
      setSelectedPromptName(initial.name);
      setSelectedPromptContent(initial.content);
      setPromptDirty(false);
      return;
    }

    if (!promptDirty) {
      const matched = prompts.find((p) => p.name === selectedPromptName);
      if (matched) {
        setSelectedPromptContent(matched.content);
      }
    }
  }, [promptsQuery.data, promptDirty, selectedPromptName]);

  const filteredModels = useMemo(() => {
    const q = modelSearch.trim().toLowerCase();
    const filtered = !q
      ? models
      : models.filter((m) => m.name.toLowerCase().includes(q) || m.id.toLowerCase().includes(q));
    return filtered;
  }, [modelSearch, models]);

  const patch = useMemo<RuntimeSettingsPatch | null>(() => {
    const current = settingsQuery.data;
    if (!current) return null;
    const next: RuntimeSettingsPatch = {};

    const aiPatch: NonNullable<RuntimeSettingsPatch["ai"]> = {};
    const trimmedDefault = defaultModel.trim();
    if (trimmedDefault !== "" && trimmedDefault !== (current.ai.default_model ?? "")) {
      aiPatch.default_model = trimmedDefault;
    }

    // Job models: derive current ordered list from settings for comparison
    const currentJobModels = (() => {
      const primary = current.ai.job_model;
      const fallbacks = current.ai.job_model_fallbacks ?? [];
      return primary ? [primary, ...fallbacks] : [];
    })();
    if (JSON.stringify(jobModels) !== JSON.stringify(currentJobModels)) {
      if (jobModels.length === 0) {
        aiPatch.job_model = "";  // clear to use default
        aiPatch.job_model_fallbacks = [];
      } else {
        aiPatch.job_model = jobModels[0];
        aiPatch.job_model_fallbacks = jobModels.slice(1);
      }
    }

    // Chat models: same logic
    const currentChatModels = (() => {
      const primary = current.ai.chat_model;
      const fallbacks = current.ai.chat_model_fallbacks ?? [];
      return primary ? [primary, ...fallbacks] : [];
    })();
    if (JSON.stringify(chatModels) !== JSON.stringify(currentChatModels)) {
      if (chatModels.length === 0) {
        aiPatch.chat_model = "";  // clear to use default
        aiPatch.chat_model_fallbacks = [];
      } else {
        aiPatch.chat_model = chatModels[0];
        aiPatch.chat_model_fallbacks = chatModels.slice(1);
      }
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
    if (telegramAllowedGroupChatId.trim() !== "") {
      const parsed = Number.parseInt(telegramAllowedGroupChatId.trim(), 10);
      if (!Number.isFinite(parsed)) {
        return null;
      }
      if (parsed !== current.telegram.allowed_group_chat_id) {
        telegramPatch.allowed_group_chat_id = parsed;
      }
    }
    if (Object.keys(telegramPatch).length > 0) {
      next.telegram = telegramPatch;
    }

    return Object.keys(next).length > 0 ? next : null;
  }, [
    aiApiKey,
    defaultModel,
    jobModels,
    chatModels,
    settingsQuery.data,
    telegramAllowedGroupChatId,
    telegramChatId,
    telegramToken,
  ]);

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

  const promptUpdateMutation = useMutation({
    mutationFn: ({ name, content }: { name: string; content: string }) =>
      api.put<PromptFileView>(`/api/v1/settings/prompts/${name}`, { content }),
    onSuccess: (updated) => {
      queryClient.setQueryData<PromptListView>(["settings-prompts"], (prev) => {
        if (!prev) {
          return { prompts: [updated] };
        }
        const next = prev.prompts.map((prompt) =>
          prompt.name === updated.name ? updated : prompt,
        );
        if (!next.some((prompt) => prompt.name === updated.name)) {
          next.push(updated);
        }
        return { prompts: next };
      });
      setSelectedPromptName(updated.name);
      setSelectedPromptContent(updated.content);
      setPromptDirty(false);
      setToast({ kind: "success", message: `Prompt saved: ${updated.name}` });
    },
    onError: (e: unknown) => {
      const message = e instanceof Error ? e.message : "Failed to update prompt";
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

  const handleSavePrompt = () => {
    if (!selectedPromptName) {
      setToast({ kind: "error", message: "Please select a prompt file." });
      return;
    }
    promptUpdateMutation.mutate({
      name: selectedPromptName,
      content: selectedPromptContent,
    });
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
      setToast({ kind: "success", message: `Fetched ${loaded.length} models.` });
    } catch (e: unknown) {
      const message = e instanceof Error ? e.message : "Failed to fetch models";
      setModelsError(message);
    } finally {
      setModelsLoading(false);
    }
  }, [aiApiKey, settingsQuery.data?.ai.openrouter_api_key]);

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

  /** Resolve model name from id */
  const modelName = useCallback(
    (id: string): string => {
      const found = models.find((m) => m.id === id);
      return found ? found.name : id;
    },
    [models],
  );

  /** Resolve model context length from id */
  const modelContext = useCallback(
    (id: string): number | null => {
      const found = models.find((m) => m.id === id);
      return found?.contextLength ?? null;
    },
    [models],
  );

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
  const availablePrompts = promptsQuery.data?.prompts ?? [];
  const selectedPromptMeta = availablePrompts.find((p) => p.name === selectedPromptName);
  const isDialogOpen = selectedSetting !== null;

  const dialogTitle =
    selectedSetting === "ai"
      ? "AI (OpenRouter)"
      : selectedSetting === "agent"
        ? "Agent Personality"
        : "Telegram Bot";

  /** Render the global default model selector (single-select) */
  const renderDefaultModelSelector = () => {
    // Sort so the currently selected model appears first
    const sorted = [...filteredModels].sort(
      (a, b) => Number(b.id === defaultModel) - Number(a.id === defaultModel),
    );

    return (
      <div className="space-y-2 rounded-lg border bg-muted/30 p-3">
        <div className="flex items-center justify-between">
          <p className="text-sm font-semibold">Default Model</p>
          <span className="text-xs text-muted-foreground">
            Active: {defaultModel || "openai/gpt-4o"}
          </span>
        </div>
        <div className="max-h-48 overflow-y-auto rounded border bg-background">
          {sorted.length === 0 && (
            <div className="p-3 text-sm text-muted-foreground">
              No models loaded. Fetch models above.
            </div>
          )}
          {sorted.map((model) => (
            <div
              key={model.id}
              className="flex items-center justify-between gap-3 border-b px-3 py-2 last:border-b-0"
            >
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{model.name}</p>
                <p className="truncate text-xs text-muted-foreground">
                  {model.id}
                  {model.contextLength ? ` -- ${Math.round(model.contextLength / 1000)}K` : ""}
                </p>
              </div>
              <Switch
                checked={model.id === defaultModel}
                onCheckedChange={(checked) => {
                  if (checked) setDefaultModel(model.id);
                }}
              />
            </div>
          ))}
        </div>
      </div>
    );
  };

  /** Unified multi-select ordered model picker for scenario-specific models */
  const renderModelPicker = (
    label: string,
    selectedModels: string[],
    setSelectedModels: React.Dispatch<React.SetStateAction<string[]>>,
  ) => {
    const isUsingDefault = selectedModels.length === 0;
    const activeDisplay = isUsingDefault
      ? (defaultModel || "openai/gpt-4o")
      : selectedModels[0];
    const selectedSet = new Set(selectedModels);

    // Available models: sort selected first, then the rest
    const sorted = [...filteredModels].sort((a, b) => {
      const aSelected = selectedSet.has(a.id) ? 0 : 1;
      const bSelected = selectedSet.has(b.id) ? 0 : 1;
      return aSelected - bSelected;
    });

    const moveUp = (index: number) => {
      if (index === 0) return;
      setSelectedModels((prev) => {
        const next = [...prev];
        [next[index - 1], next[index]] = [next[index], next[index - 1]];
        return next;
      });
    };

    const moveDown = (index: number) => {
      setSelectedModels((prev) => {
        if (index >= prev.length - 1) return prev;
        const next = [...prev];
        [next[index], next[index + 1]] = [next[index + 1], next[index]];
        return next;
      });
    };

    const remove = (index: number) => {
      setSelectedModels((prev) => prev.filter((_, i) => i !== index));
    };

    const toggleModel = (modelId: string, checked: boolean) => {
      if (checked) {
        setSelectedModels((prev) => [...prev, modelId]);
      } else {
        setSelectedModels((prev) => prev.filter((id) => id !== modelId));
      }
    };

    return (
      <div className="space-y-2 rounded-lg border bg-muted/30 p-3">
        <div className="flex items-center justify-between">
          <p className="text-sm font-semibold">{label}</p>
          <span className="text-xs text-muted-foreground">
            Active: {activeDisplay}
          </span>
        </div>

        {/* Use default model toggle */}
        <div className="flex items-center justify-between gap-3 rounded border bg-background px-3 py-2">
          <div className="min-w-0 flex-1">
            <p className="text-sm font-medium">Use default model</p>
            <p className="text-xs text-muted-foreground">
              Falls back to: {defaultModel || "openai/gpt-4o"}
            </p>
          </div>
          <Switch
            checked={isUsingDefault}
            onCheckedChange={(checked) => {
              if (checked) setSelectedModels([]);
            }}
          />
        </div>

        {/* Selected models (ordered list) */}
        {selectedModels.length > 0 && (
          <div className="space-y-1 rounded border bg-muted/20 p-2">
            <p className="text-xs font-semibold text-muted-foreground">
              Selected Models (ordered)
            </p>
            {selectedModels.map((id, index) => {
              const ctx = modelContext(id);
              return (
                <div
                  key={id}
                  className="flex items-center gap-2 rounded border bg-background px-2 py-1.5"
                >
                  <span className="w-5 shrink-0 text-center text-xs font-medium text-muted-foreground">
                    {index + 1}.
                  </span>
                  <Badge
                    variant={index === 0 ? "default" : "secondary"}
                    className="shrink-0 text-[10px] px-1.5 py-0"
                  >
                    {index === 0 ? "Primary" : "Fallback"}
                  </Badge>
                  <div className="min-w-0 flex-1">
                    <p className="truncate text-sm font-medium">{modelName(id)}</p>
                    <p className="truncate text-xs text-muted-foreground">
                      {id}{ctx ? ` -- ${Math.round(ctx / 1000)}K` : ""}
                    </p>
                  </div>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 shrink-0"
                    disabled={index === 0}
                    onClick={() => moveUp(index)}
                    title="Move up"
                  >
                    <ArrowUp className="h-3 w-3" />
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 shrink-0"
                    disabled={index === selectedModels.length - 1}
                    onClick={() => moveDown(index)}
                    title="Move down"
                  >
                    <ArrowDown className="h-3 w-3" />
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    className="h-6 w-6 shrink-0 text-muted-foreground hover:text-destructive"
                    onClick={() => remove(index)}
                    title="Remove"
                  >
                    <X className="h-3 w-3" />
                  </Button>
                </div>
              );
            })}
          </div>
        )}

        {/* Available models list */}
        <div className="max-h-48 overflow-y-auto rounded border bg-background">
          {sorted.length === 0 && (
            <div className="p-3 text-sm text-muted-foreground">
              No models loaded. Fetch models above.
            </div>
          )}
          {sorted.map((model) => (
            <div
              key={model.id}
              className="flex items-center justify-between gap-3 border-b px-3 py-2 last:border-b-0"
            >
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium">{model.name}</p>
                <p className="truncate text-xs text-muted-foreground">
                  {model.id}
                  {model.contextLength ? ` -- ${Math.round(model.contextLength / 1000)}K` : ""}
                </p>
              </div>
              <Switch
                checked={selectedSet.has(model.id)}
                onCheckedChange={(checked) => toggleModel(model.id, checked)}
              />
            </div>
          ))}
        </div>
      </div>
    );
  };

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
                Default: {current.ai.default_model ?? "Not set"} · Key:{" "}
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
          onClick={() => openSetting("agent")}
        >
          <div className="flex items-center gap-3">
            <Bot className="h-4 w-4 text-muted-foreground" />
            <div className="space-y-1">
              <p className="font-medium">Agent Personality</p>
              <p className="text-xs text-muted-foreground">
                Soul: agent/soul.md
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Badge variant="default">Active</Badge>
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
                {current.telegram.token_hint ?? "Not set"} · Group:{" "}
                {current.telegram.allowed_group_chat_id ?? "Not set"}
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
                  Showing {filteredModels.length} models
                </p>
                {modelsError && (
                  <p className="text-sm text-destructive">{modelsError}</p>
                )}

                {renderDefaultModelSelector()}

                {renderModelPicker(
                  "Job Analysis Models",
                  jobModels,
                  setJobModels,
                )}

                {renderModelPicker(
                  "Chat Models",
                  chatModels,
                  setChatModels,
                )}
              </div>
            </div>
          )}

          {selectedSetting === "agent" && (
            <div className="space-y-6 px-6 py-5">
              <div className="space-y-3 rounded-xl border bg-card p-4">
                <div className="flex items-center justify-between">
                  <Label className="text-base font-semibold">Prompt Markdown</Label>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={() => promptsQuery.refetch()}
                    disabled={promptsQuery.isFetching}
                  >
                    {promptsQuery.isFetching ? "Refreshing..." : "Refresh"}
                  </Button>
                </div>
                <Select
                  value={selectedPromptName}
                  onValueChange={(value) => {
                    const prompt = availablePrompts.find((p) => p.name === value);
                    setSelectedPromptName(value);
                    setSelectedPromptContent(prompt?.content ?? "");
                    setPromptDirty(false);
                  }}
                >
                  <SelectTrigger>
                    <SelectValue placeholder="Select prompt file" />
                  </SelectTrigger>
                  <SelectContent>
                    {availablePrompts.map((prompt) => (
                      <SelectItem key={prompt.name} value={prompt.name}>
                        {prompt.name}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <Textarea
                  value={selectedPromptContent}
                  onChange={(e) => {
                    setSelectedPromptContent(e.target.value);
                    setPromptDirty(true);
                  }}
                  rows={14}
                  className="font-mono text-xs"
                  placeholder="Prompt markdown content..."
                  disabled={!selectedPromptName}
                />
                <p className="text-xs text-muted-foreground">
                  {selectedPromptMeta?.description ??
                    "Select a prompt file to edit."}
                </p>
                <div className="flex justify-end">
                  <Button
                    type="button"
                    onClick={handleSavePrompt}
                    disabled={
                      !selectedPromptName ||
                      !promptDirty ||
                      promptUpdateMutation.isPending
                    }
                  >
                    {promptUpdateMutation.isPending ? "Saving..." : "Save Prompt Markdown"}
                  </Button>
                </div>
              </div>

            </div>
          )}

          {selectedSetting === "telegram" && (
            <div className="space-y-6 px-6 py-5">
              <div className="space-y-4 rounded-xl border bg-card p-4">
                <div className="space-y-2">
                  <Label htmlFor="telegram-chat-id" className="text-base font-semibold">
                    Chat ID
                  </Label>
                  <Input
                    id="telegram-chat-id"
                    value={telegramChatId}
                    onChange={(e) => setTelegramChatId(e.target.value)}
                    placeholder="123456789"
                    className="h-11"
                  />
                </div>
                <div className="space-y-2">
                  <Label
                    htmlFor="telegram-allowed-group-chat-id"
                    className="text-base font-semibold"
                  >
                    Allowed Group Chat ID
                  </Label>
                  <Input
                    id="telegram-allowed-group-chat-id"
                    value={telegramAllowedGroupChatId}
                    onChange={(e) => setTelegramAllowedGroupChatId(e.target.value)}
                    placeholder="-1001234567890"
                    className="h-11"
                  />
                  <p className="text-xs text-muted-foreground">
                    Bot only responds in this group when mentioned. Private chats still work.
                  </p>
                </div>
                <div className="space-y-2">
                  <Label htmlFor="telegram-token" className="text-base font-semibold">
                    Bot Token
                  </Label>
                  <Input
                    id="telegram-token"
                    type="password"
                    value={telegramToken}
                    onChange={(e) => setTelegramToken(e.target.value)}
                    placeholder={current.telegram.token_hint ?? "123456:ABC..."}
                    className="h-11"
                  />
                  <p className="text-xs text-muted-foreground">
                    Current token hint: {current.telegram.token_hint ?? "Not set"}
                  </p>
                </div>
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
