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
  Bot,
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
import { Textarea } from "@/components/ui/textarea";

type SettingKey = "ai" | "agent" | "telegram";
type ToastState = { kind: "success" | "error"; message: string } | null;
type OpenRouterModel = {
  id: string;
  name: string;
  contextLength: number | null;
};

/** Sentinel value meaning "use default model" for scenario-specific selectors */
const USE_DEFAULT = "__use_default__";

function formatUpdatedAt(value: string | null): string {
  if (!value) return "Never";
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return value;
  return d.toLocaleString();
}

export default function Settings() {
  const queryClient = useQueryClient();
  const [defaultModel, setDefaultModel] = useState("");
  const [jobModel, setJobModel] = useState(USE_DEFAULT);
  const [chatModel, setChatModel] = useState(USE_DEFAULT);
  const [aiApiKey, setAiApiKey] = useState("");
  const [showAiApiKey, setShowAiApiKey] = useState(false);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [models, setModels] = useState<OpenRouterModel[]>([]);
  const [telegramToken, setTelegramToken] = useState("");
  const [telegramChatId, setTelegramChatId] = useState("");
  const [agentSoul, setAgentSoul] = useState("");
  const [agentChatSystemPrompt, setAgentChatSystemPrompt] = useState("");
  const [agentProactiveEnabled, setAgentProactiveEnabled] = useState(false);
  const [agentProactiveCron, setAgentProactiveCron] = useState("");
  const [selectedSetting, setSelectedSetting] = useState<SettingKey | null>(null);
  const [toast, setToast] = useState<ToastState>(null);

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.get<RuntimeSettingsView>("/api/v1/settings"),
  });

  useEffect(() => {
    if (!settingsQuery.data) return;
    setDefaultModel(settingsQuery.data.ai.default_model ?? "");
    setJobModel(settingsQuery.data.ai.job_model ?? USE_DEFAULT);
    setChatModel(settingsQuery.data.ai.chat_model ?? USE_DEFAULT);
    setAiApiKey(settingsQuery.data.ai.openrouter_api_key ?? "");
    setTelegramChatId(
      settingsQuery.data.telegram.chat_id == null
        ? ""
        : String(settingsQuery.data.telegram.chat_id),
    );
    setAgentSoul(settingsQuery.data.agent.soul ?? "");
    setAgentChatSystemPrompt(settingsQuery.data.agent.chat_system_prompt ?? "");
    setAgentProactiveEnabled(settingsQuery.data.agent.proactive_enabled);
    setAgentProactiveCron(settingsQuery.data.agent.proactive_cron ?? "");
  }, [settingsQuery.data]);

  /** The effective model for a scenario — resolves "use default" to the actual default model. */
  const effectiveModel = useCallback(
    (scenarioValue: string): string => {
      if (scenarioValue === USE_DEFAULT || scenarioValue === "") {
        return defaultModel || "openai/gpt-4o";
      }
      return scenarioValue;
    },
    [defaultModel],
  );

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

    // Job model: USE_DEFAULT means send empty string (clear), otherwise send the model id
    const currentJobModel = current.ai.job_model ?? USE_DEFAULT;
    if (jobModel !== currentJobModel) {
      if (jobModel === USE_DEFAULT) {
        aiPatch.job_model = ""; // empty string clears it on the backend
      } else {
        aiPatch.job_model = jobModel;
      }
    }

    // Chat model: same logic
    const currentChatModel = current.ai.chat_model ?? USE_DEFAULT;
    if (chatModel !== currentChatModel) {
      if (chatModel === USE_DEFAULT) {
        aiPatch.chat_model = "";
      } else {
        aiPatch.chat_model = chatModel;
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
    if (Object.keys(telegramPatch).length > 0) {
      next.telegram = telegramPatch;
    }

    const agentPatch: NonNullable<RuntimeSettingsPatch["agent"]> = {};
    const trimmedSoul = agentSoul.trim();
    const currentSoul = current.agent.soul ?? "";
    if (trimmedSoul !== currentSoul) {
      agentPatch.soul = trimmedSoul || null;
    }
    const trimmedChatSystemPrompt = agentChatSystemPrompt.trim();
    const currentChatSystemPrompt = current.agent.chat_system_prompt ?? "";
    if (trimmedChatSystemPrompt !== currentChatSystemPrompt) {
      agentPatch.chat_system_prompt = trimmedChatSystemPrompt || null;
    }
    if (agentProactiveEnabled !== current.agent.proactive_enabled) {
      agentPatch.proactive_enabled = agentProactiveEnabled;
    }
    const trimmedCron = agentProactiveCron.trim();
    const currentCron = current.agent.proactive_cron ?? "";
    if (trimmedCron !== currentCron) {
      agentPatch.proactive_cron = trimmedCron || null;
    }
    if (Object.keys(agentPatch).length > 0) {
      next.agent = agentPatch;
    }

    return Object.keys(next).length > 0 ? next : null;
  }, [aiApiKey, defaultModel, jobModel, chatModel, settingsQuery.data, telegramChatId, telegramToken, agentSoul, agentChatSystemPrompt, agentProactiveEnabled, agentProactiveCron]);

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
    selectedSetting === "ai"
      ? "AI (OpenRouter)"
      : selectedSetting === "agent"
        ? "Agent Personality"
        : "Telegram Bot";

  /** Render a model selector section (shared UI for default, job, chat) */
  const renderModelSelector = (
    label: string,
    value: string,
    onChange: (id: string) => void,
    showUseDefault: boolean,
  ) => {
    const isDefault = value === USE_DEFAULT;
    const activeModelId = isDefault ? effectiveModel(value) : value;

    // Sort so the currently selected model appears first
    const sorted = [...filteredModels].sort(
      (a, b) => Number(b.id === activeModelId) - Number(a.id === activeModelId),
    );

    return (
      <div className="space-y-2 rounded-lg border bg-muted/30 p-3">
        <div className="flex items-center justify-between">
          <p className="text-sm font-semibold">{label}</p>
          <span className="text-xs text-muted-foreground">
            Active: {activeModelId || "openai/gpt-4o"}
          </span>
        </div>

        {showUseDefault && (
          <div className="flex items-center justify-between gap-3 rounded border bg-background px-3 py-2">
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Use default model</p>
              <p className="text-xs text-muted-foreground">
                Falls back to: {defaultModel || "openai/gpt-4o"}
              </p>
            </div>
            <Switch
              checked={isDefault}
              onCheckedChange={(checked) => {
                if (checked) onChange(USE_DEFAULT);
              }}
            />
          </div>
        )}

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
              <div className="flex items-center gap-2">
                <SlidersHorizontal className="h-3.5 w-3.5 text-muted-foreground" />
                <Switch
                  checked={!isDefault && model.id === value}
                  onCheckedChange={(checked) => {
                    if (checked) onChange(model.id);
                  }}
                />
              </div>
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
                Soul: {current.agent.soul ? (current.agent.soul.length > 40 ? `${current.agent.soul.slice(0, 40)}...` : current.agent.soul) : "Default"} · Proactive:{" "}
                {current.agent.proactive_enabled ? "On" : "Off"}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Badge variant={current.agent.proactive_enabled ? "default" : "secondary"}>
              {current.agent.proactive_enabled ? "Active" : "Inactive"}
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
                  Showing {filteredModels.length} models
                </p>
                {modelsError && (
                  <p className="text-sm text-destructive">{modelsError}</p>
                )}

                {renderModelSelector(
                  "Default Model",
                  defaultModel,
                  (id) => setDefaultModel(id),
                  false,
                )}

                {renderModelSelector(
                  "Job Analysis Model",
                  jobModel,
                  (id) => setJobModel(id),
                  true,
                )}

                {renderModelSelector(
                  "Chat Model",
                  chatModel,
                  (id) => setChatModel(id),
                  true,
                )}
              </div>
            </div>
          )}

          {selectedSetting === "agent" && (
            <div className="space-y-6 px-6 py-5">
              <div className="space-y-3 rounded-xl border bg-card p-4">
                <Label htmlFor="agent-chat-system-prompt" className="text-base font-semibold">
                  Chat System Prompt
                </Label>
                <Textarea
                  id="agent-chat-system-prompt"
                  value={agentChatSystemPrompt}
                  onChange={(e) => setAgentChatSystemPrompt(e.target.value)}
                  placeholder="You are a helpful career assistant..."
                  rows={6}
                  className="resize-y"
                />
                <p className="text-xs text-muted-foreground">
                  Default system prompt for new chat sessions. Leave empty to use the built-in default.
                </p>
              </div>

              <div className="space-y-3 rounded-xl border bg-card p-4">
                <Label htmlFor="agent-soul" className="text-base font-semibold">
                  Soul Prompt
                </Label>
                <Textarea
                  id="agent-soul"
                  value={agentSoul}
                  onChange={(e) => setAgentSoul(e.target.value)}
                  placeholder="You are a proactive job search companion. You're encouraging, data-driven, and concise..."
                  rows={6}
                  className="resize-y"
                />
                <p className="text-xs text-muted-foreground">
                  Defines the agent's personality for proactive messages. Leave empty to use the built-in default.
                </p>
              </div>

              <div className="space-y-3 rounded-xl border bg-card p-4">
                <div className="flex items-center justify-between">
                  <Label htmlFor="agent-proactive" className="text-base font-semibold">
                    Proactive Messaging
                  </Label>
                  <Switch
                    id="agent-proactive"
                    checked={agentProactiveEnabled}
                    onCheckedChange={setAgentProactiveEnabled}
                  />
                </div>
                <p className="text-xs text-muted-foreground">
                  When enabled, the agent periodically reviews recent chat activity and sends
                  encouraging Telegram messages when it spots something worth mentioning.
                </p>
              </div>

              <div className="space-y-3 rounded-xl border bg-card p-4">
                <Label htmlFor="agent-cron" className="text-base font-semibold">
                  Proactive Schedule (Cron)
                </Label>
                <Input
                  id="agent-cron"
                  value={agentProactiveCron}
                  onChange={(e) => setAgentProactiveCron(e.target.value)}
                  placeholder="0 9,18,21 * * *"
                />
                <p className="text-xs text-muted-foreground">
                  5-field cron expression. Changes take effect after service restart.
                  Common values: <code className="rounded bg-muted px-1">0 9 * * *</code> (daily 9 AM),{" "}
                  <code className="rounded bg-muted px-1">0 9,18,21 * * *</code> (3x daily),{" "}
                  <code className="rounded bg-muted px-1">0 */6 * * *</code> (every 6 hours).
                </p>
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
