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

import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { settingsApi } from "@/api/client";
import type {
  SettingsMap,
} from "@/api/types";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Badge } from "@/components/ui/badge";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Bot,
  BookOpen,
  ChevronRight,
  Circle,
  ExternalLink,
  Eye,
  EyeOff,
  Mail,
  MessageSquare,
  Save,
  Settings2,
  Shield,
  Sparkles,
  Users,
  Sun,
  Palette,
  Wifi,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { useTheme, type Theme } from "@/hooks/use-theme";
import { useServerStatus } from "@/hooks/use-server-status";
import { getBackendUrl, setBackendUrl } from "@/api/client";
import Skills from "@/pages/Skills";
import Agents from "@/pages/Agents";
import McpServers from "@/pages/McpServers";

type SettingsPage = "general" | "providers" | "agents" | "skills" | "mcp" | "channels" | "tools" | "security";
type ToastState = { kind: "success" | "error"; message: string } | null;

// Well-known setting keys (must match backend keys module)
const KEYS = {
  // Global defaults
  LLM_DEFAULT_PROVIDER: "llm.default_provider",
  LLM_DEFAULT_MODEL: "llm.default_model",
  // Provider: OpenRouter
  LLM_PROVIDERS_OPENROUTER_ENABLED: "llm.providers.openrouter.enabled",
  LLM_PROVIDERS_OPENROUTER_API_KEY: "llm.providers.openrouter.api_key",
  LLM_PROVIDERS_OPENROUTER_BASE_URL: "llm.providers.openrouter.base_url",
  // Provider: Ollama
  LLM_PROVIDERS_OLLAMA_ENABLED: "llm.providers.ollama.enabled",
  LLM_PROVIDERS_OLLAMA_API_KEY: "llm.providers.ollama.api_key",
  LLM_PROVIDERS_OLLAMA_BASE_URL: "llm.providers.ollama.base_url",
  // Provider: Codex
  LLM_PROVIDERS_CODEX_ENABLED: "llm.providers.codex.enabled",
  LLM_PROVIDERS_CODEX_API_KEY: "llm.providers.codex.api_key",
  LLM_PROVIDERS_CODEX_BASE_URL: "llm.providers.codex.base_url",
  // Provider: GLM (Zhipu)
  LLM_PROVIDERS_GLM_ENABLED: "llm.providers.glm.enabled",
  LLM_PROVIDERS_GLM_API_KEY: "llm.providers.glm.api_key",
  LLM_PROVIDERS_GLM_BASE_URL: "llm.providers.glm.base_url",
  // Model assignments
  LLM_MODELS_DEFAULT: "llm.models.default",
  LLM_MODELS_CHAT: "llm.models.chat",
  LLM_MODELS_JOB: "llm.models.job",
  LLM_FALLBACK_MODELS: "llm.fallback_models",
  LLM_FAVORITE_MODELS: "llm.favorite_models",
  TELEGRAM_BOT_TOKEN: "telegram.bot_token",
  TELEGRAM_CHAT_ID: "telegram.chat_id",
  TELEGRAM_ALLOWED_GROUP_CHAT_ID: "telegram.allowed_group_chat_id",
  TELEGRAM_NOTIFICATION_CHANNEL_ID: "telegram.notification_channel_id",
  GMAIL_ADDRESS: "gmail.address",
  GMAIL_APP_PASSWORD: "gmail.app_password",
  GMAIL_AUTO_SEND_ENABLED: "gmail.auto_send_enabled",
  COMPOSIO_API_KEY: "composio.api_key",
  COMPOSIO_ENTITY_ID: "composio.entity_id",
  MEMORY_MEM0_BASE_URL: "memory.mem0.base_url",
  MEMORY_MEMOS_BASE_URL: "memory.memos.base_url",
  MEMORY_MEMOS_TOKEN: "memory.memos.token",
  MEMORY_HINDSIGHT_BASE_URL: "memory.hindsight.base_url",
  MEMORY_HINDSIGHT_BANK_ID: "memory.hindsight.bank_id",
  FS_ALLOWED_DIRECTORIES: "fs.allowed_directories",
  FS_READ_ONLY_DIRECTORIES: "fs.read_only_directories",
  FS_DENIED_DIRECTORIES: "fs.denied_directories",
} as const;

const THEME_OPTIONS: Array<{ key: Theme; label: string; icon: ReactNode; description: string }> = [
  { key: "light", label: "Light", icon: <Sun className="h-4 w-4" />, description: "Bright workspace" },
];

// Sensitive keys that should be masked by default
const SENSITIVE_KEYS: Set<string> = new Set([
  KEYS.LLM_PROVIDERS_OPENROUTER_API_KEY,
  KEYS.LLM_PROVIDERS_OLLAMA_API_KEY,
  KEYS.LLM_PROVIDERS_CODEX_API_KEY,
  KEYS.LLM_PROVIDERS_GLM_API_KEY,
  KEYS.TELEGRAM_BOT_TOKEN,
  KEYS.GMAIL_APP_PASSWORD,
  KEYS.COMPOSIO_API_KEY,
  KEYS.MEMORY_MEMOS_TOKEN,
]);

// Provider definitions for the multi-provider architecture
const PROVIDER_DEFS = [
  {
    id: "openrouter",
    name: "OpenRouter",
    description: "Unified API gateway for 100+ models",
    apiKeyKey: KEYS.LLM_PROVIDERS_OPENROUTER_API_KEY,
    fields: [
      { key: KEYS.LLM_PROVIDERS_OPENROUTER_API_KEY, label: "API Key", placeholder: "sk-or-v1-...", sensitive: true },
      { key: KEYS.LLM_PROVIDERS_OPENROUTER_BASE_URL, label: "Base URL", placeholder: "https://openrouter.ai/api/v1" },
    ],
  },
  {
    id: "ollama",
    name: "Ollama",
    description: "Local model inference server",
    apiKeyKey: KEYS.LLM_PROVIDERS_OLLAMA_API_KEY,
    fields: [
      { key: KEYS.LLM_PROVIDERS_OLLAMA_API_KEY, label: "API Key", placeholder: "ollama", sensitive: true },
      { key: KEYS.LLM_PROVIDERS_OLLAMA_BASE_URL, label: "Base URL", placeholder: "http://localhost:11434/v1" },
    ],
  },
  {
    id: "codex",
    name: "Codex",
    description: "ChatGPT backend API",
    apiKeyKey: KEYS.LLM_PROVIDERS_CODEX_API_KEY,
    fields: [
      { key: KEYS.LLM_PROVIDERS_CODEX_API_KEY, label: "API Key", placeholder: "sk-...", sensitive: true },
      { key: KEYS.LLM_PROVIDERS_CODEX_BASE_URL, label: "Base URL", placeholder: "https://api.openai.com/v1" },
    ],
  },
  {
    id: "glm",
    name: "GLM (Zhipu)",
    description: "Zhipu AI large language models",
    apiKeyKey: KEYS.LLM_PROVIDERS_GLM_API_KEY,
    fields: [
      { key: KEYS.LLM_PROVIDERS_GLM_API_KEY, label: "API Key", placeholder: "zhipu-...", sensitive: true },
      { key: KEYS.LLM_PROVIDERS_GLM_BASE_URL, label: "Base URL", placeholder: "https://open.bigmodel.cn/api/paas/v4" },
    ],
  },
] as const;

// A single KV field with optional show/hide toggle for sensitive values
function KvField({
  settingKey,
  label,
  value,
  placeholder,
  onChange,
  sensitive,
  description,
}: {
  settingKey: string;
  label: string;
  value: string;
  placeholder?: string;
  onChange: (value: string) => void;
  sensitive?: boolean;
  description?: string;
}) {
  const [visible, setVisible] = useState(false);

  return (
    <div className="space-y-1.5">
      <Label htmlFor={settingKey} className="text-sm font-medium">
        {label}
      </Label>
      <div className="flex items-center gap-2">
        <Input
          id={settingKey}
          type={sensitive && !visible ? "password" : "text"}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={placeholder}
          className="h-9 font-mono text-sm"
        />
        {sensitive && (
          <Button
            type="button"
            variant="outline"
            size="icon"
            className="h-9 w-9 shrink-0"
            onClick={() => setVisible((v) => !v)}
          >
            {visible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
          </Button>
        )}
      </div>
      {description && (
        <p className="text-xs text-muted-foreground">{description}</p>
      )}
    </div>
  );
}

// Group of KV fields with a save button
function KvGroup({
  title,
  description,
  icon,
  fields,
  values,
  original,
  onFieldChange,
  onSave,
  saving,
  toast,
}: {
  title: string;
  description: string;
  icon: ReactNode;
  fields: Array<{
    key: string;
    label: string;
    placeholder?: string;
    description?: string;
  }>;
  values: Record<string, string>;
  original: Record<string, string>;
  onFieldChange: (key: string, value: string) => void;
  onSave: () => void;
  saving: boolean;
  toast: ToastState;
}) {
  const hasChanges = fields.some((f) => (values[f.key] ?? "") !== (original[f.key] ?? ""));

  return (
    <Card className="app-surface border-border/60">
      <CardHeader className="pb-4">
        <div className="flex items-center gap-3">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
            {icon}
          </div>
          <div>
            <CardTitle className="text-base">{title}</CardTitle>
            <CardDescription>{description}</CardDescription>
          </div>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        {fields.map((f) => (
          <KvField
            key={f.key}
            settingKey={f.key}
            label={f.label}
            value={values[f.key] ?? ""}
            placeholder={f.placeholder}
            onChange={(v) => onFieldChange(f.key, v)}
            sensitive={SENSITIVE_KEYS.has(f.key)}
            description={f.description}
          />
        ))}
        <div className="flex items-center justify-between pt-2">
          <div>
            {toast && (
              <p className={cn("text-sm", toast.kind === "success" ? "text-green-600" : "text-destructive")}>
                {toast.message}
              </p>
            )}
          </div>
          <Button
            onClick={onSave}
            disabled={!hasChanges || saving}
            size="sm"
          >
            <Save className="mr-1.5 h-3.5 w-3.5" />
            {saving ? "Saving..." : "Save"}
          </Button>
        </div>
      </CardContent>
    </Card>
  );
}

/** Backend URL configuration card for the General settings tab. */
function ConnectionCard() {
  const { isOnline } = useServerStatus();
  const [url, setUrl] = useState(() => getBackendUrl());
  const [saving, setSaving] = useState(false);
  const [result, setResult] = useState<{ kind: "success" | "error"; message: string } | null>(null);

  async function saveAndReconnect() {
    setSaving(true);
    setResult(null);
    try {
      const res = await fetch(`${url}/api/v1/settings`, {
        signal: AbortSignal.timeout(5000),
      });
      if (res.ok) {
        setBackendUrl(url); // persists + reloads
      } else {
        setResult({ kind: "error", message: `Server returned ${res.status}` });
      }
    } catch (e) {
      setResult({ kind: "error", message: `Cannot connect: ${e instanceof Error ? e.message : String(e)}` });
    } finally {
      setSaving(false);
    }
  }

  return (
    <Card className="app-surface border-border/60">
      <CardHeader>
        <div className="flex items-center gap-3">
          <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
            <Wifi className="h-4 w-4" />
          </div>
          <div className="flex-1">
            <CardTitle className="text-base">Connection</CardTitle>
            <CardDescription>Backend server URL</CardDescription>
          </div>
          <Badge variant={isOnline ? "secondary" : "destructive"} className="capitalize">
            {isOnline ? "Connected" : "Disconnected"}
          </Badge>
        </div>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="flex items-center gap-2">
          <Input
            value={url}
            onChange={(e) => { setUrl(e.target.value); setResult(null); }}
            placeholder="http://localhost:25555"
            className="h-9 font-mono text-sm"
            onKeyDown={(e) => {
              if (e.key === "Enter" && !saving) saveAndReconnect();
            }}
          />
          <Button
            onClick={saveAndReconnect}
            disabled={saving || !url.trim()}
            size="sm"
          >
            <Save className="mr-1.5 h-3.5 w-3.5" />
            {saving ? "Testing..." : "Save & Reconnect"}
          </Button>
        </div>
        {result && (
          <p className={cn("text-sm", result.kind === "success" ? "text-green-600" : "text-destructive")}>
            {result.message}
          </p>
        )}
      </CardContent>
    </Card>
  );
}

/** Status badge for provider cards — green dot for connected, gray for not configured. */
function ProviderStatusBadge({ configured }: { configured: boolean }) {
  return configured ? (
    <span className="inline-flex items-center gap-1.5 text-xs font-medium text-green-600">
      <Circle className="h-2 w-2 fill-green-500 text-green-500" />
      Connected
    </span>
  ) : (
    <span className="inline-flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
      <Circle className="h-2 w-2 fill-muted-foreground/40 text-muted-foreground/40" />
      Not configured
    </span>
  );
}

/** Collapsible provider card — collapsed shows name + status, expanded shows fields + save. */
function ProviderCard({
  provider,
  draft,
  original,
  onFieldChange,
  onGroupSave,
  saving,
  toast,
}: {
  provider: (typeof PROVIDER_DEFS)[number];
  draft: Record<string, string>;
  original: Record<string, string>;
  onFieldChange: (key: string, value: string) => void;
  onGroupSave: (keys: string[], group: string) => void;
  saving: boolean;
  toast: ToastState;
}) {
  const [expanded, setExpanded] = useState(false);
  const hasApiKey = !!(draft[provider.apiKeyKey] ?? "").trim();
  const allKeys = provider.fields.map((f) => f.key);
  const groupId = `provider-${provider.id}`;
  const hasChanges = allKeys.some((k) => (draft[k] ?? "") !== (original[k] ?? ""));

  return (
    <div className="border rounded-lg bg-card">
      {/* Header — always visible */}
      <button
        type="button"
        onClick={() => setExpanded((v) => !v)}
        className="w-full flex items-center justify-between p-4 text-left hover:bg-accent/30 transition-colors rounded-lg"
      >
        <div className="flex items-center gap-3">
          <ChevronRight
            className={cn(
              "h-4 w-4 text-muted-foreground transition-transform duration-200",
              expanded && "rotate-90"
            )}
          />
          <div>
            <h3 className="text-sm font-medium">{provider.name}</h3>
            <p className="text-xs text-muted-foreground">{provider.description}</p>
          </div>
        </div>
        <ProviderStatusBadge configured={hasApiKey} />
      </button>

      {/* Body — collapsible */}
      {expanded && (
        <div className="px-4 pb-4 space-y-3 border-t">
          <div className="pt-3 space-y-3">
            {provider.fields.map((field) => (
              <KvField
                key={field.key}
                settingKey={field.key}
                label={field.label}
                value={draft[field.key] ?? ""}
                placeholder={field.placeholder}
                onChange={(v) => onFieldChange(field.key, v)}
                sensitive={SENSITIVE_KEYS.has(field.key)}
              />
            ))}
          </div>
          <div className="flex items-center justify-between pt-1">
            <div>
              {toast && (
                <p className={cn("text-sm", toast.kind === "success" ? "text-green-600" : "text-destructive")}>
                  {toast.message}
                </p>
              )}
            </div>
            <Button
              onClick={(e) => { e.stopPropagation(); onGroupSave(allKeys, groupId); }}
              disabled={!hasChanges || saving}
              size="sm"
            >
              <Save className="mr-1.5 h-3.5 w-3.5" />
              {saving ? "Saving..." : "Save"}
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

/** Providers settings tab — default model + collapsible provider cards + model assignments. */
function ProvidersSection({
  draft,
  original,
  onFieldChange,
  onGroupSave,
  saving,
  groupToasts,
}: {
  draft: Record<string, string>;
  original: Record<string, string>;
  onFieldChange: (key: string, value: string) => void;
  onGroupSave: (keys: string[], group: string) => void;
  saving: boolean;
  groupToasts: Record<string, ToastState>;
}) {
  const defaultModelChanged = (draft[KEYS.LLM_DEFAULT_MODEL] ?? "") !== (original[KEYS.LLM_DEFAULT_MODEL] ?? "");

  return (
    <>
      {/* Default Model */}
      <Card className="app-surface border-border/60">
        <CardHeader className="pb-4">
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
              <Sparkles className="h-4 w-4" />
            </div>
            <div>
              <CardTitle className="text-base">Default Model</CardTitle>
              <CardDescription>Used when no specific model is configured</CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-4">
          <KvField
            settingKey={KEYS.LLM_DEFAULT_MODEL}
            label="Model"
            value={draft[KEYS.LLM_DEFAULT_MODEL] ?? ""}
            placeholder="openai/gpt-4o-mini"
            onChange={(v) => onFieldChange(KEYS.LLM_DEFAULT_MODEL, v)}
          />
          <div className="flex items-center justify-between pt-2">
            <div>
              {groupToasts["global-defaults"] && (
                <p className={cn("text-sm", groupToasts["global-defaults"]!.kind === "success" ? "text-green-600" : "text-destructive")}>
                  {groupToasts["global-defaults"]!.message}
                </p>
              )}
            </div>
            <Button
              onClick={() => onGroupSave([KEYS.LLM_DEFAULT_MODEL], "global-defaults")}
              disabled={!defaultModelChanged || saving}
              size="sm"
            >
              <Save className="mr-1.5 h-3.5 w-3.5" />
              {saving ? "Saving..." : "Save"}
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Provider Cards */}
      <Card className="app-surface border-border/60">
        <CardHeader className="pb-4">
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
              <Sparkles className="h-4 w-4" />
            </div>
            <div>
              <CardTitle className="text-base">Providers</CardTitle>
              <CardDescription>Configure API keys for LLM providers</CardDescription>
            </div>
          </div>
        </CardHeader>
        <CardContent className="space-y-2">
          {PROVIDER_DEFS.map((provider) => (
            <ProviderCard
              key={provider.id}
              provider={provider}
              draft={draft}
              original={original}
              onFieldChange={onFieldChange}
              onGroupSave={onGroupSave}
              saving={saving}
              toast={groupToasts[`provider-${provider.id}`] ?? null}
            />
          ))}
        </CardContent>
      </Card>

      {/* Model Assignments */}
      <KvGroup
        title="Model Assignments"
        description="Map model keys to specific model IDs. Unset keys fall back to default."
        icon={<Bot className="h-4 w-4" />}
        fields={[
          { key: KEYS.LLM_MODELS_DEFAULT, label: "Default Model", placeholder: "openai/gpt-4o" },
          { key: KEYS.LLM_MODELS_CHAT, label: "Chat Model", placeholder: "(falls back to default)", description: "Model used for interactive chat" },
          { key: KEYS.LLM_MODELS_JOB, label: "Job Model", placeholder: "(falls back to default)", description: "Model used for job analysis tasks" },
          { key: KEYS.LLM_FALLBACK_MODELS, label: "Fallback Models", placeholder: "model1,model2,model3", description: "Comma-separated ordered fallback list" },
          { key: KEYS.LLM_FAVORITE_MODELS, label: "Favorite Models", placeholder: "model1,model2", description: "Comma-separated favorites shown in model picker" },
        ]}
        values={draft}
        original={original}
        onFieldChange={onFieldChange}
        onSave={() => onGroupSave([KEYS.LLM_MODELS_DEFAULT, KEYS.LLM_MODELS_CHAT, KEYS.LLM_MODELS_JOB, KEYS.LLM_FALLBACK_MODELS, KEYS.LLM_FAVORITE_MODELS], "llm-models")}
        saving={saving}
        toast={groupToasts["llm-models"] ?? null}
      />
    </>
  );
}

export default function Settings() {
  const [searchParams, setSearchParams] = useSearchParams();
  const { theme, setTheme } = useTheme();
  const queryClient = useQueryClient();
  const [activeCategory, setActiveCategory] = useState<SettingsPage>(() => {
    const section = searchParams.get("section");
    const allowed: SettingsPage[] = ["general", "providers", "agents", "skills", "mcp", "channels", "tools", "security"];
    return allowed.includes(section as SettingsPage) ? (section as SettingsPage) : "general";
  });
  const [toast, setToast] = useState<ToastState>(null);

  // Local draft of all KV values
  const [draft, setDraft] = useState<Record<string, string>>({});

  // Group-level toasts
  const [groupToasts, setGroupToasts] = useState<Record<string, ToastState>>({});

  // Fetch all settings as flat KV map
  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => settingsApi.list(),
  });

  // Sync fetched data into draft
  useEffect(() => {
    if (!settingsQuery.data) return;
    setDraft((prev) => {
      const next = { ...settingsQuery.data };
      // Preserve local edits for keys that haven't been saved yet
      for (const [k, v] of Object.entries(prev)) {
        if (v !== (settingsQuery.data![k] ?? "") && v !== "") {
          next[k] = v;
        }
      }
      return next;
    });
  }, [settingsQuery.data]);

  // Toast auto-dismiss
  useEffect(() => {
    if (!toast) return;
    const timer = window.setTimeout(() => setToast(null), 3000);
    return () => window.clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    for (const [group, t] of Object.entries(groupToasts)) {
      if (!t) continue;
      const timer = window.setTimeout(() => {
        setGroupToasts((prev) => ({ ...prev, [group]: null }));
      }, 3000);
      return () => window.clearTimeout(timer);
    }
  }, [groupToasts]);

  // Batch save mutation for a group of keys
  const saveMutation = useMutation({
    mutationFn: async ({ keys, group }: { keys: string[]; group: string }) => {
      const patches: Record<string, string | null> = {};
      const original = settingsQuery.data ?? {};
      for (const key of keys) {
        const newVal = draft[key] ?? "";
        const oldVal = original[key] ?? "";
        if (newVal !== oldVal) {
          patches[key] = newVal || null; // empty string = delete
        }
      }
      if (Object.keys(patches).length === 0) return group;
      await settingsApi.batchUpdate(patches);
      return group;
    },
    onSuccess: (group) => {
      queryClient.invalidateQueries({ queryKey: ["settings"] });
      setGroupToasts((prev) => ({ ...prev, [group]: { kind: "success", message: "Settings saved." } }));
    },
    onError: (e: unknown, { group }) => {
      const message = e instanceof Error ? e.message : "Failed to save settings";
      setGroupToasts((prev) => ({ ...prev, [group]: { kind: "error", message } }));
    },
  });

  const handleFieldChange = (key: string, value: string) => {
    setDraft((prev) => ({ ...prev, [key]: value }));
  };

  const handleGroupSave = (keys: string[], group: string) => {
    saveMutation.mutate({ keys, group });
  };

  if (settingsQuery.isLoading) {
    return (
      <div className="space-y-6">
        <Skeleton className="h-10 w-56" />
        <Skeleton className="h-72 w-full" />
      </div>
    );
  }

  if (settingsQuery.isError) {
    return (
      <div className="space-y-4">
        <h1 className="text-2xl font-bold">Settings</h1>
        <p className="text-sm text-destructive">
          Failed to load settings. Please refresh and try again.
        </p>
      </div>
    );
  }

  const original: SettingsMap = settingsQuery.data ?? {};

  const settingsNavItems: Array<{
    id: SettingsPage;
    label: string;
    icon: ReactNode;
    summary: string;
  }> = [
    { id: "general", label: "General", icon: <Palette className="h-4 w-4" />, summary: "Appearance and documentation" },
    { id: "providers", label: "Providers", icon: <Sparkles className="h-4 w-4" />, summary: "LLM provider and model config" },
    { id: "agents", label: "Agents", icon: <Users className="h-4 w-4" />, summary: "Agent definitions and overrides" },
    { id: "skills", label: "Skills", icon: <Bot className="h-4 w-4" />, summary: "Installed skills and management" },
    { id: "mcp", label: "MCP Servers", icon: <ExternalLink className="h-4 w-4" />, summary: "Tool server connections" },
    { id: "channels", label: "Channels", icon: <MessageSquare className="h-4 w-4" />, summary: "Telegram, Gmail" },
    { id: "tools", label: "Tools", icon: <Settings2 className="h-4 w-4" />, summary: "Composio, memory integrations" },
    { id: "security", label: "Security", icon: <Shield className="h-4 w-4" />, summary: "Filesystem sandbox" },
  ];

  return (
    <div className="flex h-full gap-4 overflow-hidden">
      {/* Sidebar */}
      <aside className="data-panel w-64 shrink-0 overflow-y-auto">
        <div className="border-b border-border/70 px-4 py-4">
          <h1 className="text-lg font-semibold tracking-tight">Settings</h1>
          <p className="mt-1 text-xs text-muted-foreground">
            Configure runtime credentials and workspace behavior.
          </p>
        </div>
        <div className="p-2">
          <nav className="space-y-1">
            {settingsNavItems.map((item) => (
              <button
                key={item.id}
                type="button"
                onClick={() => {
                  setActiveCategory(item.id);
                  setSearchParams({ section: item.id });
                }}
                className={cn(
                  "group flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left text-sm transition-all",
                  activeCategory === item.id
                    ? "bg-primary/10 text-foreground shadow-sm ring-1 ring-primary/15"
                    : "text-muted-foreground hover:bg-background/70 hover:text-foreground hover:ring-1 hover:ring-border/70"
                )}
              >
                <span
                  className={cn(
                    "inline-flex h-7 w-7 items-center justify-center rounded-lg border border-border/70 bg-background/70",
                    activeCategory === item.id
                      ? "text-primary"
                      : "text-muted-foreground group-hover:text-foreground"
                  )}
                >
                  {item.icon}
                </span>
                <span className="min-w-0">
                  <span className="block truncate font-medium">{item.label}</span>
                  <span className="block truncate text-xs text-muted-foreground/80">{item.summary}</span>
                </span>
              </button>
            ))}
          </nav>
        </div>
      </aside>

      {/* Content */}
      <div className="flex-1 space-y-6 overflow-y-auto pr-1">
        {/* Toast */}
        {toast && (
          <div className={cn(
            "rounded-lg border px-4 py-2 text-sm",
            toast.kind === "success" ? "border-green-200 bg-green-50 text-green-700" : "border-destructive/30 bg-destructive/5 text-destructive"
          )}>
            {toast.message}
          </div>
        )}

        {/* ── General ── */}
        {activeCategory === "general" && (
          <>
            {/* Connection */}
            <ConnectionCard />

            {/* Appearance */}
            <Card className="app-surface border-border/60">
              <CardHeader>
                <div className="flex items-center gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
                    <Palette className="h-4 w-4" />
                  </div>
                  <div>
                    <CardTitle className="text-base">Appearance</CardTitle>
                    <CardDescription>Theme and display preferences</CardDescription>
                  </div>
                </div>
              </CardHeader>
              <CardContent>
                <div className="mb-3 flex items-center justify-between gap-3">
                  <div className="space-y-1">
                    <p className="font-medium">Theme</p>
                    <p className="text-xs text-muted-foreground">
                      Choose how the UI looks across all pages.
                    </p>
                  </div>
                  <Badge variant="secondary" className="capitalize">{theme}</Badge>
                </div>
                <div className="grid gap-2 md:grid-cols-3">
                  {THEME_OPTIONS.map((option) => (
                    <button
                      key={option.key}
                      type="button"
                      onClick={() => setTheme(option.key)}
                      className={cn(
                        "group rounded-xl border p-3 text-left transition-all",
                        theme === option.key
                          ? "border-primary/30 bg-primary/8 shadow-sm ring-1 ring-primary/10"
                          : "hover:bg-accent/40"
                      )}
                    >
                      <div className="flex items-center gap-2">
                        <span className={cn(
                          "inline-flex h-8 w-8 items-center justify-center rounded-lg border",
                          theme === option.key
                            ? "border-primary/20 bg-primary/10 text-primary"
                            : "border-border/70 bg-background/70 text-muted-foreground"
                        )}>
                          {option.icon}
                        </span>
                        <div className="min-w-0">
                          <p className="text-sm font-medium">{option.label}</p>
                          <p className="truncate text-xs text-muted-foreground">{option.description}</p>
                        </div>
                      </div>
                    </button>
                  ))}
                </div>
              </CardContent>
            </Card>

            {/* Documentation */}
            <Card className="app-surface border-border/60">
              <CardHeader>
                <div className="flex items-center gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
                    <BookOpen className="h-4 w-4" />
                  </div>
                  <div>
                    <CardTitle className="text-base">Documentation</CardTitle>
                    <CardDescription>Project guides and backend API reference</CardDescription>
                  </div>
                </div>
              </CardHeader>
              <CardContent>
                <div className="grid gap-2 lg:grid-cols-2">
                  <a
                    href="/book/"
                    target="_blank"
                    rel="noreferrer"
                    className="group rounded-xl border bg-card p-4 transition-colors hover:bg-accent/30"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex items-center gap-3">
                        <BookOpen className="h-4 w-4 text-muted-foreground" />
                        <div>
                          <p className="font-medium">Guides</p>
                          <p className="text-xs text-muted-foreground">mdBook</p>
                        </div>
                      </div>
                      <ExternalLink className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5" />
                    </div>
                  </a>
                </div>
              </CardContent>
            </Card>
          </>
        )}

        {/* ── Providers ── */}
        {activeCategory === "providers" && (
          <ProvidersSection
            draft={draft}
            original={original}
            onFieldChange={handleFieldChange}
            onGroupSave={handleGroupSave}
            saving={saveMutation.isPending}
            groupToasts={groupToasts}
          />
        )}

        {/* ── Agents ── */}
        {activeCategory === "agents" && (
          <div className="data-panel p-4">
            <Agents />
          </div>
        )}

        {/* ── Skills ── */}
        {activeCategory === "skills" && (
          <div className="data-panel p-4">
            <Skills />
          </div>
        )}

        {/* ── MCP Servers ── */}
        {activeCategory === "mcp" && (
          <div className="data-panel p-4">
            <McpServers />
          </div>
        )}

        {/* ── Channels ── */}
        {activeCategory === "channels" && (
          <>
            <KvGroup
              title="Telegram"
              description="Bot token and chat IDs for Telegram integration"
              icon={<MessageSquare className="h-4 w-4" />}
              fields={[
                { key: KEYS.TELEGRAM_BOT_TOKEN, label: "Bot Token", placeholder: "123456:ABC-DEF..." },
                { key: KEYS.TELEGRAM_CHAT_ID, label: "Chat ID", placeholder: "e.g. 123456789" },
                { key: KEYS.TELEGRAM_ALLOWED_GROUP_CHAT_ID, label: "Allowed Group Chat ID", placeholder: "e.g. -100123456789" },
                { key: KEYS.TELEGRAM_NOTIFICATION_CHANNEL_ID, label: "Notification Channel ID", placeholder: "e.g. -100123456789" },
              ]}
              values={draft}
              original={original}
              onFieldChange={handleFieldChange}
              onSave={() => handleGroupSave([KEYS.TELEGRAM_BOT_TOKEN, KEYS.TELEGRAM_CHAT_ID, KEYS.TELEGRAM_ALLOWED_GROUP_CHAT_ID, KEYS.TELEGRAM_NOTIFICATION_CHANNEL_ID], "telegram")}
              saving={saveMutation.isPending}
              toast={groupToasts["telegram"] ?? null}
            />

            <KvGroup
              title="Gmail"
              description="SMTP credentials for sending emails"
              icon={<Mail className="h-4 w-4" />}
              fields={[
                { key: KEYS.GMAIL_ADDRESS, label: "Email Address", placeholder: "you@gmail.com" },
                { key: KEYS.GMAIL_APP_PASSWORD, label: "App Password", placeholder: "xxxx xxxx xxxx xxxx" },
                { key: KEYS.GMAIL_AUTO_SEND_ENABLED, label: "Auto-Send Enabled", placeholder: "true or false", description: "Set to 'true' to enable auto-sending" },
              ]}
              values={draft}
              original={original}
              onFieldChange={handleFieldChange}
              onSave={() => handleGroupSave([KEYS.GMAIL_ADDRESS, KEYS.GMAIL_APP_PASSWORD, KEYS.GMAIL_AUTO_SEND_ENABLED], "gmail")}
              saving={saveMutation.isPending}
              toast={groupToasts["gmail"] ?? null}
            />
          </>
        )}

        {/* ── Tools ── */}
        {activeCategory === "tools" && (
          <>
            <KvGroup
              title="Composio"
              description="Tool orchestration platform credentials"
              icon={<Settings2 className="h-4 w-4" />}
              fields={[
                { key: KEYS.COMPOSIO_API_KEY, label: "API Key", placeholder: "cmp-..." },
                { key: KEYS.COMPOSIO_ENTITY_ID, label: "Entity ID", placeholder: "default" },
              ]}
              values={draft}
              original={original}
              onFieldChange={handleFieldChange}
              onSave={() => handleGroupSave([KEYS.COMPOSIO_API_KEY, KEYS.COMPOSIO_ENTITY_ID], "composio")}
              saving={saveMutation.isPending}
              toast={groupToasts["composio"] ?? null}
            />
            <KvGroup
              title="Memory"
              description="External memory service connections"
              icon={<Bot className="h-4 w-4" />}
              fields={[
                { key: KEYS.MEMORY_MEM0_BASE_URL, label: "Mem0 Base URL", placeholder: "http://localhost:..." },
                { key: KEYS.MEMORY_MEMOS_BASE_URL, label: "Memos Base URL", placeholder: "http://localhost:5230" },
                { key: KEYS.MEMORY_MEMOS_TOKEN, label: "Memos Token" },
                { key: KEYS.MEMORY_HINDSIGHT_BASE_URL, label: "Hindsight Base URL", placeholder: "http://localhost:..." },
                { key: KEYS.MEMORY_HINDSIGHT_BANK_ID, label: "Hindsight Bank ID" },
              ]}
              values={draft}
              original={original}
              onFieldChange={handleFieldChange}
              onSave={() => handleGroupSave([KEYS.MEMORY_MEM0_BASE_URL, KEYS.MEMORY_MEMOS_BASE_URL, KEYS.MEMORY_MEMOS_TOKEN, KEYS.MEMORY_HINDSIGHT_BASE_URL, KEYS.MEMORY_HINDSIGHT_BANK_ID], "memory")}
              saving={saveMutation.isPending}
              toast={groupToasts["memory"] ?? null}
            />
          </>
        )}

        {/* ── Security ── */}
        {activeCategory === "security" && (
          <>
            <KvGroup
              title="Filesystem Sandbox"
              description="Control which directories agents can access. Values are JSON arrays of directory paths."
              icon={<Shield className="h-4 w-4" />}
              fields={[
                { key: KEYS.FS_ALLOWED_DIRECTORIES, label: "Allowed Directories (Read/Write)", placeholder: '["/tmp/workspace", "/data/shared"]', description: "Directories where agents can read and write files" },
                { key: KEYS.FS_READ_ONLY_DIRECTORIES, label: "Read-Only Directories", placeholder: '["/etc/config"]', description: "Directories where agents can only read files" },
                { key: KEYS.FS_DENIED_DIRECTORIES, label: "Denied Directories", placeholder: '["/etc/secrets", "/root"]', description: "Directories that agents are explicitly blocked from accessing" },
              ]}
              values={draft}
              original={original}
              onFieldChange={handleFieldChange}
              onSave={() => {
                const fsKeys = [KEYS.FS_ALLOWED_DIRECTORIES, KEYS.FS_READ_ONLY_DIRECTORIES, KEYS.FS_DENIED_DIRECTORIES];
                for (const key of fsKeys) {
                  const val = (draft[key] ?? "").trim();
                  if (val === "") continue;
                  try {
                    const parsed = JSON.parse(val);
                    if (!Array.isArray(parsed) || !parsed.every((v: unknown) => typeof v === "string")) {
                      setToast({ kind: "error", message: `Invalid value for ${key}: must be a JSON array of strings.` });
                      return;
                    }
                  } catch {
                    setToast({ kind: "error", message: `Invalid JSON for ${key}. Expected a JSON array like ["/path/a", "/path/b"].` });
                    return;
                  }
                }
                handleGroupSave(fsKeys, "fs-sandbox");
              }}
              saving={saveMutation.isPending}
              toast={groupToasts["fs-sandbox"] ?? null}
            />

            {/* Current Status */}
            <Card className="app-surface border-border/60">
              <CardHeader className="pb-4">
                <div className="flex items-center gap-3">
                  <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
                    <Shield className="h-4 w-4" />
                  </div>
                  <div>
                    <CardTitle className="text-base">Current Status</CardTitle>
                    <CardDescription>Active filesystem access rules</CardDescription>
                  </div>
                </div>
              </CardHeader>
              <CardContent>
                {(() => {
                  const parseJsonArray = (key: string): string[] => {
                    const val = (original[key] ?? "").trim();
                    if (!val) return [];
                    try {
                      const parsed = JSON.parse(val);
                      if (Array.isArray(parsed)) return parsed.filter((v): v is string => typeof v === "string");
                    } catch { /* ignore */ }
                    return [];
                  };
                  const allowed = parseJsonArray(KEYS.FS_ALLOWED_DIRECTORIES);
                  const readOnly = parseJsonArray(KEYS.FS_READ_ONLY_DIRECTORIES);
                  const denied = parseJsonArray(KEYS.FS_DENIED_DIRECTORIES);
                  const hasAny = allowed.length > 0 || readOnly.length > 0 || denied.length > 0;

                  if (!hasAny) {
                    return (
                      <p className="text-sm text-muted-foreground">
                        No restrictions configured — agents have unrestricted file access.
                      </p>
                    );
                  }

                  return (
                    <div className="space-y-3">
                      {allowed.length > 0 && (
                        <div className="space-y-1.5">
                          <p className="text-xs font-medium text-muted-foreground">Allowed (Read/Write)</p>
                          <div className="flex flex-wrap gap-1.5">
                            {allowed.map((dir) => (
                              <Badge key={dir} variant="outline" className="border-green-300 bg-green-50 text-green-700">
                                {dir}
                              </Badge>
                            ))}
                          </div>
                        </div>
                      )}
                      {readOnly.length > 0 && (
                        <div className="space-y-1.5">
                          <p className="text-xs font-medium text-muted-foreground">Read-Only</p>
                          <div className="flex flex-wrap gap-1.5">
                            {readOnly.map((dir) => (
                              <Badge key={dir} variant="outline" className="border-amber-300 bg-amber-50 text-amber-700">
                                {dir}
                              </Badge>
                            ))}
                          </div>
                        </div>
                      )}
                      {denied.length > 0 && (
                        <div className="space-y-1.5">
                          <p className="text-xs font-medium text-muted-foreground">Denied</p>
                          <div className="flex flex-wrap gap-1.5">
                            {denied.map((dir) => (
                              <Badge key={dir} variant="outline" className="border-red-300 bg-red-50 text-red-700">
                                {dir}
                              </Badge>
                            ))}
                          </div>
                        </div>
                      )}
                    </div>
                  );
                })()}
              </CardContent>
            </Card>
          </>
        )}

      </div>

    </div>
  );
}
