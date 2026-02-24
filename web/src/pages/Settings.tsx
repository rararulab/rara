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
  CreateContactRequest,
  PullProgressEvent,
  PromptFileView,
  PromptListView,
  RuntimeSettingsPatch,
  RuntimeSettingsView,
  TelegramContact,
  UpdateContactRequest,
} from "@/api/types";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
  Mail,
  MessageSquare,
  Pencil,
  Plus,
  Search,
  Sparkles,
  RefreshCw,
  Trash2,
  Users,
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

type SettingKey = "ai" | "agent" | "telegram" | "composio" | "contacts";
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

const BASE_URL = import.meta.env.VITE_API_URL || '';

export default function Settings() {
  const queryClient = useQueryClient();
  const [defaultModel, setDefaultModel] = useState("");
  const [jobModels, setJobModels] = useState<string[]>([]);   // ordered: [primary, fallback1, fallback2, ...]
  const [chatModels, setChatModels] = useState<string[]>([]); // ordered: [primary, fallback1, fallback2, ...]
  const [aiProvider, setAiProvider] = useState("openrouter");
  const [ollamaBaseUrl, setOllamaBaseUrl] = useState("http://localhost:11434");
  const [aiApiKey, setAiApiKey] = useState("");
  const [showAiApiKey, setShowAiApiKey] = useState(false);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelsError, setModelsError] = useState<string | null>(null);
  const [modelSearch, setModelSearch] = useState("");
  const [models, setModels] = useState<OpenRouterModel[]>([]);
  const [telegramToken, setTelegramToken] = useState("");
  const [telegramChatId, setTelegramChatId] = useState("");
  const [telegramAllowedGroupChatId, setTelegramAllowedGroupChatId] = useState("");
  const [telegramNotificationChannelId, setTelegramNotificationChannelId] = useState("");
  const [composioApiKey, setComposioApiKey] = useState("");
  const [showComposioApiKey, setShowComposioApiKey] = useState(false);
  const [composioEntityId, setComposioEntityId] = useState("");
  const [selectedPromptName, setSelectedPromptName] = useState("");
  const [selectedPromptContent, setSelectedPromptContent] = useState("");
  const [promptDirty, setPromptDirty] = useState(false);
  const [selectedSetting, setSelectedSetting] = useState<SettingKey | null>(null);
  const [toast, setToast] = useState<ToastState>(null);

  // -- ollama pull state --
  const [pullModelName, setPullModelName] = useState("");
  const [pullProgress, setPullProgress] = useState<{ status: string; pct: number } | null>(null);
  const [pullError, setPullError] = useState<string | null>(null);

  // -- ollama capability filter --
  const [capabilityFilter, setCapabilityFilter] = useState<Set<string>>(new Set());

  // -- contacts state --
  const [contactDialogOpen, setContactDialogOpen] = useState(false);
  const [editingContact, setEditingContact] = useState<TelegramContact | null>(null);
  const [contactName, setContactName] = useState("");
  const [contactUsername, setContactUsername] = useState("");
  const [contactNotes, setContactNotes] = useState("");
  const [contactEnabled, setContactEnabled] = useState(true);

  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => api.get<RuntimeSettingsView>("/api/v1/settings"),
  });


  const promptsQuery = useQuery({
    queryKey: ["settings-prompts"],
    queryFn: () => api.get<PromptListView>("/api/v1/settings/prompts"),
  });

  const contactsQuery = useQuery({
    queryKey: ["contacts"],
    queryFn: () => api.get<TelegramContact[]>("/api/v1/contacts"),
  });

  const { data: recommendations, isLoading: recsLoading, refetch: refetchRecs } = useQuery({
    queryKey: ["ollama-recommendations"],
    queryFn: () => api.getOllamaModelRecommendations(),
    enabled: false,
    staleTime: 5 * 60 * 1000,
  });

  const ollamaHealthQuery = useQuery({
    queryKey: ["ollama-health"],
    queryFn: () => api.ollamaHealth(),
    enabled: aiProvider === "ollama",
    refetchInterval: 30_000,
    retry: false,
  });

  const ollamaModelsQuery = useQuery({
    queryKey: ["ollama-models"],
    queryFn: () => api.ollamaListModels(),
    enabled: aiProvider === "ollama" && ollamaHealthQuery.data?.healthy === true,
  });

  const ollamaDeleteMutation = useMutation({
    mutationFn: (name: string) => api.ollamaDeleteModel(name),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["ollama-models"] });
      setToast({ kind: "success", message: "Model deleted." });
    },
    onError: (e: unknown) => {
      const message = e instanceof Error ? e.message : "Failed to delete model";
      setToast({ kind: "error", message });
    },
  });

  const ollamaModels = ollamaModelsQuery.data?.models ?? [];

  // Collect all unique capabilities across all models
  const allCapabilities = useMemo(() => {
    const caps = new Set<string>();
    for (const model of ollamaModels) {
      for (const cap of model.capabilities) {
        caps.add(cap);
      }
    }
    return Array.from(caps).sort();
  }, [ollamaModels]);

  // Filter models by selected capabilities (intersection — model must have ALL selected caps)
  const filteredOllamaModels = useMemo(() => {
    if (capabilityFilter.size === 0) return ollamaModels;
    return ollamaModels.filter((model) =>
      Array.from(capabilityFilter).every((cap) => model.capabilities.includes(cap))
    );
  }, [ollamaModels, capabilityFilter]);

  const handlePullModel = async () => {
    const name = pullModelName.trim();
    if (!name) return;
    setPullProgress({ status: "Starting...", pct: 0 });
    setPullError(null);

    try {
      const res = await fetch(`${BASE_URL}/api/v1/settings/ollama/models/pull`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ name }),
      });

      if (!res.ok) {
        const text = await res.text();
        setPullError(text || "Pull failed");
        setPullProgress(null);
        return;
      }

      const reader = res.body?.getReader();
      const decoder = new TextDecoder();
      if (!reader) {
        setPullError("No response body");
        setPullProgress(null);
        return;
      }

      let buffer = "";
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });

        const lines = buffer.split("\n");
        buffer = lines.pop() ?? "";

        for (const line of lines) {
          if (!line.startsWith("data: ")) continue;
          const json = line.slice(6).trim();
          if (!json) continue;
          try {
            const event = JSON.parse(json) as PullProgressEvent;
            if (event.type === "progress") {
              const pct =
                event.total && event.total > 0
                  ? Math.round(((event.completed ?? 0) / event.total) * 100)
                  : 0;
              setPullProgress({ status: event.status, pct });
            } else if (event.type === "done") {
              setPullProgress(null);
              setPullModelName("");
              queryClient.invalidateQueries({ queryKey: ["ollama-models"] });
              setToast({
                kind: "success",
                message: `Model "${name}" pulled successfully.`,
              });
            } else if (event.type === "error") {
              setPullError(event.message);
              setPullProgress(null);
            }
          } catch {
            // skip malformed JSON
          }
        }
      }
    } catch (e) {
      setPullError(e instanceof Error ? e.message : "Pull failed");
      setPullProgress(null);
    }
  };

  const createContactMutation = useMutation({
    mutationFn: (req: CreateContactRequest) =>
      api.post<TelegramContact>("/api/v1/contacts", req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
      setContactDialogOpen(false);
      setToast({ kind: "success", message: "Contact created." });
    },
    onError: (e: unknown) => {
      const message = e instanceof Error ? e.message : "Failed to create contact";
      setToast({ kind: "error", message });
    },
  });

  const updateContactMutation = useMutation({
    mutationFn: ({ id, req }: { id: string; req: UpdateContactRequest }) =>
      api.put<TelegramContact>(`/api/v1/contacts/${id}`, req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
      setContactDialogOpen(false);
      setToast({ kind: "success", message: "Contact updated." });
    },
    onError: (e: unknown) => {
      const message = e instanceof Error ? e.message : "Failed to update contact";
      setToast({ kind: "error", message });
    },
  });

  const deleteContactMutation = useMutation({
    mutationFn: (id: string) => api.del(`/api/v1/contacts/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
      setToast({ kind: "success", message: "Contact deleted." });
    },
    onError: (e: unknown) => {
      const message = e instanceof Error ? e.message : "Failed to delete contact";
      setToast({ kind: "error", message });
    },
  });

  const openNewContact = () => {
    setEditingContact(null);
    setContactName("");
    setContactUsername("");
    setContactNotes("");
    setContactEnabled(true);
    setContactDialogOpen(true);
  };

  const openEditContact = (contact: TelegramContact) => {
    setEditingContact(contact);
    setContactName(contact.name);
    setContactUsername(contact.telegram_username);
    setContactNotes(contact.notes ?? "");
    setContactEnabled(contact.enabled);
    setContactDialogOpen(true);
  };

  const handleSaveContact = () => {
    if (editingContact) {
      updateContactMutation.mutate({
        id: editingContact.id,
        req: {
          name: contactName.trim() || undefined,
          telegram_username: contactUsername.trim() || undefined,
          notes: contactNotes.trim() || null,
          enabled: contactEnabled,
        },
      });
    } else {
      createContactMutation.mutate({
        name: contactName.trim(),
        telegram_username: contactUsername.trim(),
        notes: contactNotes.trim() || undefined,
        enabled: contactEnabled,
      });
    }
  };

  useEffect(() => {
    if (!settingsQuery.data) return;
    setAiProvider(settingsQuery.data.ai.provider ?? "openrouter");
    setOllamaBaseUrl(settingsQuery.data.ai.ollama_base_url ?? "http://localhost:11434");
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
    setTelegramNotificationChannelId(
      settingsQuery.data.telegram.notification_channel_id == null
        ? ""
        : String(settingsQuery.data.telegram.notification_channel_id),
    );
    setComposioApiKey(settingsQuery.data.agent.composio.api_key ?? "");
    setComposioEntityId(settingsQuery.data.agent.composio.entity_id ?? "");
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
    const currentProvider = current.ai.provider ?? "openrouter";
    if (aiProvider !== currentProvider) {
      aiPatch.provider = aiProvider;
    }
    const currentOllamaUrl = current.ai.ollama_base_url ?? "http://localhost:11434";
    if (ollamaBaseUrl.trim() !== currentOllamaUrl) {
      aiPatch.ollama_base_url = ollamaBaseUrl.trim();
    }
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

    if (aiProvider === "openrouter" && aiApiKey.trim() !== "") {
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
    // notification_channel_id: empty string means clear (send null), number means set
    const trimmedNotifChannel = telegramNotificationChannelId.trim();
    if (trimmedNotifChannel === "") {
      // User cleared the field — if currently set, send null to clear it
      if (current.telegram.notification_channel_id != null) {
        telegramPatch.notification_channel_id = null;
      }
    } else {
      const parsed = Number.parseInt(trimmedNotifChannel, 10);
      if (!Number.isFinite(parsed)) {
        return null;
      }
      if (parsed !== current.telegram.notification_channel_id) {
        telegramPatch.notification_channel_id = parsed;
      }
    }
    if (Object.keys(telegramPatch).length > 0) {
      next.telegram = telegramPatch;
    }

    const agentPatch: NonNullable<RuntimeSettingsPatch["agent"]> = {};
    const currentComposioApiKey = current.agent.composio.api_key ?? "";
    const currentComposioEntityId = current.agent.composio.entity_id ?? "";
    const nextComposioApiKey = composioApiKey.trim();
    const nextComposioEntityId = composioEntityId.trim();

    if (nextComposioApiKey !== currentComposioApiKey) {
      // empty string means clear
      agentPatch.composio = {
        ...(agentPatch.composio ?? {}),
        api_key: nextComposioApiKey,
      };
    }
    if (nextComposioEntityId !== currentComposioEntityId) {
      agentPatch.composio = {
        ...(agentPatch.composio ?? {}),
        entity_id: nextComposioEntityId,
      };
    }
    if (Object.keys(agentPatch).length > 0) {
      next.agent = agentPatch;
    }

    return Object.keys(next).length > 0 ? next : null;
  }, [
    aiProvider,
    ollamaBaseUrl,
    aiApiKey,
    composioApiKey,
    composioEntityId,
    defaultModel,
    jobModels,
    chatModels,
    settingsQuery.data,
    telegramAllowedGroupChatId,
    telegramChatId,
    telegramNotificationChannelId,
    telegramToken,
  ]);

  const updateMutation = useMutation({
    mutationFn: (payload: RuntimeSettingsPatch) =>
      api.post<RuntimeSettingsView>("/api/v1/settings", payload),
    onSuccess: (updated) => {
      queryClient.setQueryData(["settings"], updated);
      setAiApiKey(updated.ai.openrouter_api_key ?? "");
      setShowAiApiKey(false);
      setComposioApiKey(updated.agent.composio.api_key ?? "");
      setComposioEntityId(updated.agent.composio.entity_id ?? "");
      setShowComposioApiKey(false);
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
    } else if (setting === "composio") {
      setShowComposioApiKey(false);
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
    if (aiProvider !== "openrouter") return;
    if (!settingsQuery.data?.ai.openrouter_api_key) return;
    if (models.length > 0) return;
    if (modelsLoading) return;
    void fetchModels();
  }, [
    aiProvider,
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

  const providerLabel = (current.ai.provider ?? "openrouter") === "ollama" ? "Ollama" : "OpenRouter";

  const dialogTitle =
    selectedSetting === "ai"
      ? `AI Provider (${providerLabel})`
      : selectedSetting === "composio"
        ? "Composio"
      : selectedSetting === "agent"
        ? "Agent Personality"
        : selectedSetting === "contacts"
          ? "Telegram Contacts"
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
              <p className="font-medium">AI Provider ({providerLabel})</p>
              <p className="text-xs text-muted-foreground">
                Default: {current.ai.default_model ?? "Not set"}
                {providerLabel === "OpenRouter" && <> · Key: {current.ai.openrouter_api_key ? "Set" : "Not set"}</>}
                {providerLabel === "Ollama" && <> · URL: {current.ai.ollama_base_url ?? "localhost:11434"}</>}
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
          onClick={() => openSetting("composio")}
        >
          <div className="flex items-center gap-3">
            <Sparkles className="h-4 w-4 text-muted-foreground" />
            <div className="space-y-1">
              <p className="font-medium">Composio</p>
              <p className="text-xs text-muted-foreground">
                Entity: {current.agent.composio.entity_id ?? "default"} · Key:{" "}
                {current.agent.composio.api_key ? "Set" : "Not set"}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Badge variant={current.agent.composio.api_key ? "default" : "secondary"}>
              {current.agent.composio.api_key ? "Configured" : "Not configured"}
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
                {current.telegram.allowed_group_chat_id ?? "Not set"} · Notif Channel:{" "}
                {current.telegram.notification_channel_id ?? "Not set"}
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

        <button
          type="button"
          className="flex w-full items-center justify-between rounded-lg border p-4 text-left transition-colors hover:bg-accent"
          onClick={() => openSetting("contacts")}
        >
          <div className="flex items-center gap-3">
            <Users className="h-4 w-4 text-muted-foreground" />
            <div className="space-y-1">
              <p className="font-medium">Telegram Contacts</p>
              <p className="text-xs text-muted-foreground">
                {contactsQuery.data?.length ?? 0} contacts ·{" "}
                {contactsQuery.data?.filter((c) => c.enabled).length ?? 0} enabled
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <Badge variant={(contactsQuery.data?.length ?? 0) > 0 ? "default" : "secondary"}>
              {(contactsQuery.data?.length ?? 0) > 0 ? `${contactsQuery.data!.length} contacts` : "No contacts"}
            </Badge>
            <ChevronRight className="h-4 w-4 text-muted-foreground" />
          </div>
        </button>

      </div>

      {/* ── Gmail Status ─────────────────────────────────────── */}
      {current.gmail && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Mail className="h-4 w-4" />
              Gmail Integration
            </CardTitle>
            <CardDescription>
              Gmail account used for sending job applications.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="grid gap-4 sm:grid-cols-3">
              <div className="space-y-1 rounded-lg border bg-muted/30 p-3">
                <p className="text-xs font-medium text-muted-foreground">Status</p>
                <div className="mt-1">
                  <Badge variant={current.gmail.configured ? "default" : "secondary"}>
                    {current.gmail.configured ? "Configured" : "Not configured"}
                  </Badge>
                </div>
              </div>
              <div className="space-y-1 rounded-lg border bg-muted/30 p-3">
                <p className="text-xs font-medium text-muted-foreground">Email Address</p>
                <p className="text-sm font-medium">
                  {current.gmail.address ?? "Not set"}
                </p>
                {current.gmail.app_password_hint && (
                  <p className="text-xs text-muted-foreground">
                    Password: {current.gmail.app_password_hint}
                  </p>
                )}
              </div>
              <div className="space-y-1 rounded-lg border bg-muted/30 p-3">
                <p className="text-xs font-medium text-muted-foreground">Auto-Send</p>
                <div className="mt-1">
                  <Badge variant={current.gmail.auto_send_enabled ? "default" : "secondary"}>
                    {current.gmail.auto_send_enabled ? "Enabled" : "Disabled"}
                  </Badge>
                </div>
              </div>
            </div>
          </CardContent>
        </Card>
      )}

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
              {/* Provider selector */}
              <div className="space-y-3 rounded-xl border bg-card p-4">
                <Label className="text-base font-semibold">Provider</Label>
                <Select value={aiProvider} onValueChange={setAiProvider}>
                  <SelectTrigger className="h-11">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="openrouter">OpenRouter (Cloud)</SelectItem>
                    <SelectItem value="ollama">Ollama (Local)</SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-xs text-muted-foreground">
                  {aiProvider === "ollama"
                    ? "Ollama runs models locally — no API key needed."
                    : "OpenRouter provides access to many cloud LLM providers."}
                </p>
              </div>

              {/* OpenRouter API Key — only when provider is openrouter */}
              {aiProvider === "openrouter" && (
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
              )}

              {/* Ollama Base URL — only when provider is ollama */}
              {aiProvider === "ollama" && (
                <div className="space-y-3 rounded-xl border bg-card p-4">
                  <Label htmlFor="ollama-base-url" className="text-base font-semibold">
                    Ollama Base URL
                  </Label>
                  <Input
                    id="ollama-base-url"
                    value={ollamaBaseUrl}
                    onChange={(e) => setOllamaBaseUrl(e.target.value)}
                    placeholder="http://localhost:11434"
                    className="h-11"
                  />
                  <p className="text-xs text-muted-foreground">
                    Default: http://localhost:11434. Change if Ollama runs on a different host/port.
                  </p>
                </div>
              )}

              <div className="space-y-3 rounded-xl border bg-card p-4">
                <div className="flex items-center justify-between">
                  <Label htmlFor={aiProvider === "ollama" ? "ollama-default-model" : "models-search"} className="text-xl font-semibold">
                    Models
                  </Label>
                  {aiProvider === "openrouter" && (
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
                  )}
                </div>

                {aiProvider === "ollama" ? (
                  <div className="space-y-4">
                    {/* Health indicator */}
                    <div className="flex items-center gap-2">
                      <span className="text-sm font-medium">Status:</span>
                      {ollamaHealthQuery.isLoading ? (
                        <Skeleton className="h-5 w-28" />
                      ) : ollamaHealthQuery.data?.healthy ? (
                        <Badge variant="default" className="gap-1">
                          <span className="inline-block h-2 w-2 rounded-full bg-green-400" />
                          Connected{ollamaHealthQuery.data.version ? ` v${ollamaHealthQuery.data.version}` : ""}
                        </Badge>
                      ) : (
                        <Badge variant="destructive" className="gap-1">
                          <span className="inline-block h-2 w-2 rounded-full bg-red-400" />
                          Unreachable
                        </Badge>
                      )}
                      <Button
                        type="button"
                        variant="ghost"
                        size="icon"
                        className="h-7 w-7"
                        onClick={() => ollamaHealthQuery.refetch()}
                        disabled={ollamaHealthQuery.isFetching}
                      >
                        <RefreshCw className={`h-3 w-3 ${ollamaHealthQuery.isFetching ? "animate-spin" : ""}`} />
                      </Button>
                    </div>

                    {/* Active model display */}
                    <div className="flex items-center gap-2 rounded-lg border bg-muted/30 px-3 py-2">
                      <span className="text-sm text-muted-foreground">Active:</span>
                      <span className="text-sm font-mono font-medium">{defaultModel || "Not set"}</span>
                    </div>

                    {/* Local models list */}
                    {ollamaHealthQuery.data?.healthy && (
                      <div className="space-y-2 rounded-lg border bg-muted/30 p-3">
                        <div className="flex items-center justify-between">
                          <p className="text-sm font-semibold">Local Models</p>
                          <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            onClick={() => ollamaModelsQuery.refetch()}
                            disabled={ollamaModelsQuery.isFetching}
                          >
                            <RefreshCw className={`h-3 w-3 mr-1 ${ollamaModelsQuery.isFetching ? "animate-spin" : ""}`} />
                            Refresh
                          </Button>
                        </div>
                        {/* Capability filters */}
                        {allCapabilities.length > 0 && (
                          <div className="flex items-center gap-2 flex-wrap">
                            <span className="text-xs text-muted-foreground shrink-0">Filter:</span>
                            {allCapabilities.map((cap) => (
                              <Button
                                key={cap}
                                type="button"
                                variant={capabilityFilter.has(cap) ? "default" : "outline"}
                                size="sm"
                                className="h-6 text-xs px-2"
                                onClick={() => {
                                  setCapabilityFilter((prev) => {
                                    const next = new Set(prev);
                                    if (next.has(cap)) {
                                      next.delete(cap);
                                    } else {
                                      next.add(cap);
                                    }
                                    return next;
                                  });
                                }}
                              >
                                {cap}
                              </Button>
                            ))}
                            {capabilityFilter.size > 0 && (
                              <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                className="h-6 text-xs px-2 text-muted-foreground"
                                onClick={() => setCapabilityFilter(new Set())}
                              >
                                Clear
                              </Button>
                            )}
                          </div>
                        )}
                        <p className="text-xs text-muted-foreground">
                          Showing {filteredOllamaModels.length} of {ollamaModels.length} models
                        </p>
                        {ollamaModelsQuery.isLoading ? (
                          <div className="space-y-2">
                            <Skeleton className="h-8 w-full" />
                            <Skeleton className="h-8 w-full" />
                          </div>
                        ) : filteredOllamaModels.length === 0 ? (
                          <p className="text-sm text-muted-foreground py-2">
                            {ollamaModels.length === 0
                              ? "No models pulled yet. Pull a model below to get started."
                              : "No models match the selected filters."}
                          </p>
                        ) : (
                          <div className="max-h-56 overflow-y-auto rounded border bg-background">
                            {filteredOllamaModels.map((model) => (
                              <div
                                key={model.name}
                                className="flex items-center justify-between gap-3 border-b px-3 py-2 last:border-b-0"
                              >
                                <div className="min-w-0 flex-1">
                                  <div className="flex items-center gap-2">
                                    <p className="truncate text-sm font-medium font-mono">{model.name}</p>
                                    {model.name === defaultModel && (
                                      <Badge variant="default" className="text-[10px] px-1.5 py-0 shrink-0">Active</Badge>
                                    )}
                                  </div>
                                  <p className="text-xs text-muted-foreground">
                                    {(model.size / 1e9).toFixed(1)} GB
                                    {model.parameter_size ? ` \u00B7 ${model.parameter_size}` : ""}
                                    {model.quantization_level ? ` \u00B7 ${model.quantization_level}` : ""}
                                    {model.family ? ` \u00B7 ${model.family}` : ""}
                                  </p>
                                  {model.capabilities.length > 0 && (
                                    <div className="flex items-center gap-1 mt-0.5">
                                      {model.capabilities.map((cap) => (
                                        <Badge
                                          key={cap}
                                          variant={cap === "tools" ? "default" : "secondary"}
                                          className="text-[9px] px-1 py-0"
                                        >
                                          {cap}
                                        </Badge>
                                      ))}
                                    </div>
                                  )}
                                </div>
                                <div className="flex items-center gap-1 shrink-0">
                                  <Button
                                    type="button"
                                    variant="outline"
                                    size="sm"
                                    className="h-7 text-xs"
                                    onClick={() => setDefaultModel(model.name)}
                                  >
                                    Use
                                  </Button>
                                  <Button
                                    type="button"
                                    variant="ghost"
                                    size="icon"
                                    className="h-7 w-7 text-muted-foreground hover:text-destructive"
                                    onClick={() => {
                                      if (window.confirm(`Delete model "${model.name}"?`)) {
                                        ollamaDeleteMutation.mutate(model.name);
                                      }
                                    }}
                                    disabled={ollamaDeleteMutation.isPending}
                                    title="Delete model"
                                  >
                                    <Trash2 className="h-3 w-3" />
                                  </Button>
                                </div>
                              </div>
                            ))}
                          </div>
                        )}

                        {/* Pull model */}
                        <div className="space-y-2 pt-2 border-t">
                          <p className="text-sm font-semibold">Pull Model</p>
                          <div className="flex items-center gap-2">
                            <Input
                              value={pullModelName}
                              onChange={(e) => setPullModelName(e.target.value)}
                              placeholder="e.g. llama3.2, qwen2.5, deepseek-r1"
                              className="h-9"
                              disabled={!!pullProgress}
                            />
                            <Button
                              type="button"
                              variant="outline"
                              size="sm"
                              className="h-9 shrink-0"
                              onClick={handlePullModel}
                              disabled={!pullModelName.trim() || !!pullProgress}
                            >
                              <Download className="h-3 w-3 mr-1" />
                              Pull
                            </Button>
                          </div>
                          {pullProgress && (
                            <div className="space-y-1">
                              <div className="flex items-center justify-between text-xs text-muted-foreground">
                                <span className="truncate">{pullProgress.status}</span>
                                <span className="shrink-0">{pullProgress.pct}%</span>
                              </div>
                              <div className="h-2 w-full rounded-full bg-muted overflow-hidden">
                                <div
                                  className="h-full rounded-full bg-primary transition-all duration-300"
                                  style={{ width: `${pullProgress.pct}%` }}
                                />
                              </div>
                            </div>
                          )}
                          {pullError && (
                            <p className="text-sm text-destructive">{pullError}</p>
                          )}
                        </div>

                      </div>
                    )}
                  </div>
                ) : (
                  <>
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
                  </>
                )}
              </div>

              {/* llmfit model recommendations — on-demand */}
              {settingsQuery.data?.ai.provider === "ollama" && (
                <Card>
                  <CardHeader>
                    <div className="flex items-center justify-between">
                      <div>
                        <CardTitle className="flex items-center gap-2">
                          <Bot className="h-5 w-5" />
                          硬件感知模型推荐
                        </CardTitle>
                        <CardDescription>
                          由 llmfit 分析当前硬件，推荐适合运行的本地模型
                        </CardDescription>
                      </div>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => refetchRecs()}
                        disabled={recsLoading}
                      >
                        <RefreshCw className={`h-4 w-4 mr-1 ${recsLoading ? "animate-spin" : ""}`} />
                        {recommendations ? "刷新" : "开始分析"}
                      </Button>
                    </div>
                  </CardHeader>
                  {(recsLoading || recommendations) && (
                    <CardContent>
                      {recsLoading ? (
                        <div className="space-y-2">
                          <Skeleton className="h-8 w-full" />
                          <Skeleton className="h-8 w-full" />
                          <Skeleton className="h-8 w-full" />
                        </div>
                      ) : recommendations?.error && !recommendations.system ? (
                        <div className="text-sm text-destructive p-4 bg-muted rounded-lg">
                          <p>{recommendations.error}</p>
                        </div>
                      ) : recommendations ? (
                        <>
                          {recommendations.system && (
                            <div className="text-xs text-muted-foreground mb-3 flex gap-4 flex-wrap">
                              <span>RAM: {recommendations.system.total_ram_gb.toFixed(0)}GB</span>
                              {recommendations.system.has_gpu && (
                                <span>GPU: {recommendations.system.gpu_name} ({recommendations.system.gpu_vram_gb?.toFixed(0)}GB)</span>
                              )}
                              <span>Backend: {recommendations.system.backend}</span>
                              <span>CPU: {recommendations.system.cpu_cores} 核</span>
                            </div>
                          )}
                          <div className="space-y-2">
                            {recommendations.models.map((model, idx) => (
                              <div key={idx} className="flex items-center gap-3 p-2 rounded-lg border hover:bg-muted/50 transition-colors">
                                <div className="flex-1 min-w-0">
                                  <div className="flex items-center gap-2 flex-wrap">
                                    <span className="font-mono text-sm font-medium truncate">{model.name}</span>
                                    {model.installed && <Badge variant="outline" className="text-xs shrink-0">已安装</Badge>}
                                    <Badge
                                      className="text-xs shrink-0"
                                      variant={
                                        model.fit_level === "Perfect" ? "default" :
                                        model.fit_level === "Good" ? "secondary" :
                                        "outline"
                                      }
                                    >
                                      {model.fit_level}
                                    </Badge>
                                  </div>
                                  <div className="text-xs text-muted-foreground mt-0.5 flex gap-3 flex-wrap">
                                    <span>评分 {model.score.toFixed(0)}</span>
                                    <span>{model.estimated_tps.toFixed(0)} tok/s</span>
                                    <span>{model.best_quant}</span>
                                    <span>{model.memory_required_gb.toFixed(1)}GB</span>
                                    <span>{model.run_mode}</span>
                                  </div>
                                </div>
                                <Button
                                  size="sm"
                                  variant="outline"
                                  onClick={() => {
                                    updateMutation.mutate({ ai: { chat_model: model.name } });
                                  }}
                                >
                                  选用
                                </Button>
                              </div>
                            ))}
                          </div>
                          {recommendations.error && (
                            <p className="mt-2 text-xs text-destructive">{recommendations.error}</p>
                          )}
                        </>
                      ) : null}
                    </CardContent>
                  )}
                </Card>
              )}
            </div>
          )}

          {selectedSetting === "composio" && (
            <div className="space-y-6 px-6 py-5">
              <div className="space-y-4 rounded-xl border bg-card p-4">
                <div className="space-y-2">
                  <Label htmlFor="composio-api-key" className="text-base font-semibold">
                    Composio API Key
                  </Label>
                  <div className="flex items-center gap-2">
                    <Input
                      id="composio-api-key"
                      type={showComposioApiKey ? "text" : "password"}
                      value={composioApiKey}
                      onChange={(e) => setComposioApiKey(e.target.value)}
                      placeholder={current.agent.composio.api_key ?? "cmp_..."}
                      className="h-11"
                    />
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      className="h-11 w-11 shrink-0"
                      onClick={() => setShowComposioApiKey((v) => !v)}
                    >
                      {showComposioApiKey ? <EyeOff /> : <Eye />}
                    </Button>
                  </div>
                  <p className="text-xs text-muted-foreground">
                    Leave empty and save to clear the key from runtime settings.
                  </p>
                </div>

                <div className="space-y-2">
                  <Label htmlFor="composio-entity-id" className="text-base font-semibold">
                    Default Entity ID
                  </Label>
                  <Input
                    id="composio-entity-id"
                    value={composioEntityId}
                    onChange={(e) => setComposioEntityId(e.target.value)}
                    placeholder="default"
                    className="h-11"
                  />
                  <p className="text-xs text-muted-foreground">
                    Used when tool calls omit entity_id.
                  </p>
                </div>
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
                  <Label
                    htmlFor="telegram-notification-channel-id"
                    className="text-base font-semibold"
                  >
                    Notification Channel ID
                  </Label>
                  <Input
                    id="telegram-notification-channel-id"
                    value={telegramNotificationChannelId}
                    onChange={(e) => setTelegramNotificationChannelId(e.target.value)}
                    placeholder="-1001234567890"
                    className="h-11"
                  />
                  <p className="text-xs text-muted-foreground">
                    Telegram channel ID for automated notifications (e.g. pipeline results).
                    Leave empty to use private chat.
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

          {selectedSetting === "contacts" && (
            <div className="space-y-4 px-6 py-5">
              <div className="flex items-center justify-between">
                <p className="text-sm text-muted-foreground">
                  Manage allowed Telegram contacts. Only contacts in this list can receive messages via the send_telegram tool.
                </p>
                <Button type="button" variant="outline" size="sm" onClick={openNewContact}>
                  <Plus className="mr-1 h-3 w-3" />
                  Add
                </Button>
              </div>

              {contactsQuery.isLoading && <Skeleton className="h-32 w-full" />}

              {contactsQuery.data && contactsQuery.data.length === 0 && (
                <div className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground">
                  No contacts yet. Add a contact to enable recipient-based messaging.
                </div>
              )}

              {contactsQuery.data && contactsQuery.data.length > 0 && (
                <div className="space-y-2">
                  {contactsQuery.data.map((contact) => (
                    <div
                      key={contact.id}
                      className="flex items-center justify-between gap-3 rounded-lg border bg-card p-3"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <p className="font-medium text-sm">{contact.name}</p>
                          <Badge variant={contact.enabled ? "default" : "secondary"} className="text-[10px] px-1.5 py-0">
                            {contact.enabled ? "Enabled" : "Disabled"}
                          </Badge>
                          {contact.chat_id != null && (
                            <Badge variant="outline" className="text-[10px] px-1.5 py-0">
                              Resolved
                            </Badge>
                          )}
                        </div>
                        <p className="text-xs text-muted-foreground mt-0.5">
                          @{contact.telegram_username}
                          {contact.chat_id != null && ` · chat_id: ${contact.chat_id}`}
                          {contact.notes && ` · ${contact.notes}`}
                        </p>
                      </div>
                      <div className="flex items-center gap-1 shrink-0">
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7"
                          onClick={() => openEditContact(contact)}
                          title="Edit"
                        >
                          <Pencil className="h-3 w-3" />
                        </Button>
                        <Button
                          type="button"
                          variant="ghost"
                          size="icon"
                          className="h-7 w-7 text-muted-foreground hover:text-destructive"
                          onClick={() => {
                            if (window.confirm(`Delete contact "${contact.name}"?`)) {
                              deleteContactMutation.mutate(contact.id);
                            }
                          }}
                          title="Delete"
                        >
                          <Trash2 className="h-3 w-3" />
                        </Button>
                      </div>
                    </div>
                  ))}
                </div>
              )}

              {/* Add/Edit Contact Dialog */}
              <Dialog open={contactDialogOpen} onOpenChange={setContactDialogOpen}>
                <DialogContent className="sm:max-w-md">
                  <DialogHeader>
                    <DialogTitle>{editingContact ? "Edit Contact" : "Add Contact"}</DialogTitle>
                    <DialogDescription>
                      {editingContact
                        ? "Update this contact's details."
                        : "Add a new Telegram contact to the allowlist."}
                    </DialogDescription>
                  </DialogHeader>
                  <div className="space-y-4 py-4">
                    <div className="space-y-2">
                      <Label htmlFor="contact-name">Name</Label>
                      <Input
                        id="contact-name"
                        value={contactName}
                        onChange={(e) => setContactName(e.target.value)}
                        placeholder="John Doe"
                      />
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="contact-username">Telegram Username</Label>
                      <Input
                        id="contact-username"
                        value={contactUsername}
                        onChange={(e) => setContactUsername(e.target.value)}
                        placeholder="johndoe"
                      />
                      <p className="text-xs text-muted-foreground">Without the @ prefix.</p>
                    </div>
                    <div className="space-y-2">
                      <Label htmlFor="contact-notes">Notes</Label>
                      <Input
                        id="contact-notes"
                        value={contactNotes}
                        onChange={(e) => setContactNotes(e.target.value)}
                        placeholder="Optional notes..."
                      />
                    </div>
                    <div className="flex items-center justify-between">
                      <Label htmlFor="contact-enabled">Enabled</Label>
                      <Switch
                        id="contact-enabled"
                        checked={contactEnabled}
                        onCheckedChange={setContactEnabled}
                      />
                    </div>
                  </div>
                  <DialogFooter>
                    <Button
                      variant="outline"
                      onClick={() => setContactDialogOpen(false)}
                    >
                      Cancel
                    </Button>
                    <Button
                      onClick={handleSaveContact}
                      disabled={
                        !contactName.trim() ||
                        !contactUsername.trim() ||
                        createContactMutation.isPending ||
                        updateContactMutation.isPending
                      }
                    >
                      {createContactMutation.isPending || updateContactMutation.isPending
                        ? "Saving..."
                        : editingContact
                          ? "Update"
                          : "Create"}
                    </Button>
                  </DialogFooter>
                </DialogContent>
              </Dialog>
            </div>
          )}

          {selectedSetting !== "contacts" && (
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
          )}
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
