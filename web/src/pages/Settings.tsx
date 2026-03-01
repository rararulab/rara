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

import type { ReactNode } from "react";
import { useEffect, useState } from "react";
import { useSearchParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { settingsApi, api } from "@/api/client";
import type {
  CreateContactRequest,
  PromptListView,
  SettingsMap,
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
  Bot,
  BookOpen,
  ExternalLink,
  Eye,
  EyeOff,
  Mail,
  MessageSquare,
  Pencil,
  Plus,
  Save,
  Settings2,
  Sparkles,
  Trash2,
  Users,
  Sun,
  Moon,
  Monitor,
  Palette,
} from "lucide-react";

import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";
import { useTheme, type Theme } from "@/hooks/use-theme";
import Skills from "@/pages/Skills";
import McpServers from "@/pages/McpServers";

type SettingsPage = "general" | "providers" | "prompts" | "skills" | "mcp" | "channels" | "tools";
type ToastState = { kind: "success" | "error"; message: string } | null;

// Well-known setting keys (must match backend keys module)
const KEYS = {
  LLM_PROVIDER: "llm.provider",
  LLM_OPENROUTER_API_KEY: "llm.openrouter.api_key",
  LLM_OLLAMA_BASE_URL: "llm.ollama.base_url",
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
} as const;

const THEME_OPTIONS: Array<{ key: Theme; label: string; icon: ReactNode; description: string }> = [
  { key: "system", label: "System", icon: <Monitor className="h-4 w-4" />, description: "Follow OS appearance" },
  { key: "light", label: "Light", icon: <Sun className="h-4 w-4" />, description: "Bright workspace" },
  { key: "dark", label: "Dark", icon: <Moon className="h-4 w-4" />, description: "Low-light friendly" },
];

// Sensitive keys that should be masked by default
const SENSITIVE_KEYS: Set<string> = new Set([
  KEYS.LLM_OPENROUTER_API_KEY,
  KEYS.TELEGRAM_BOT_TOKEN,
  KEYS.GMAIL_APP_PASSWORD,
  KEYS.COMPOSIO_API_KEY,
  KEYS.MEMORY_MEMOS_TOKEN,
]);

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
      <div className="flex items-center justify-between">
        <Label htmlFor={settingKey} className="text-sm font-medium">
          {label}
        </Label>
        <span className="font-mono text-[10px] text-muted-foreground">{settingKey}</span>
      </div>
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

export default function Settings() {
  const [searchParams, setSearchParams] = useSearchParams();
  const { theme, setTheme } = useTheme();
  const queryClient = useQueryClient();
  const [activeCategory, setActiveCategory] = useState<SettingsPage>(() => {
    const section = searchParams.get("section");
    const allowed: SettingsPage[] = ["general", "providers", "prompts", "skills", "mcp", "channels", "tools"];
    return allowed.includes(section as SettingsPage) ? (section as SettingsPage) : "general";
  });
  const [toast, setToast] = useState<ToastState>(null);

  // Local draft of all KV values
  const [draft, setDraft] = useState<Record<string, string>>({});

  // Group-level toasts
  const [groupToasts, setGroupToasts] = useState<Record<string, ToastState>>({});

  // Contacts state
  const [contactDialogOpen, setContactDialogOpen] = useState(false);
  const [editingContact, setEditingContact] = useState<TelegramContact | null>(null);
  const [contactName, setContactName] = useState("");
  const [contactUsername, setContactUsername] = useState("");
  const [contactNotes, setContactNotes] = useState("");
  const [contactEnabled, setContactEnabled] = useState(true);

  // Prompt viewer state
  const [selectedPromptName, setSelectedPromptName] = useState("");
  const [selectedPromptContent, setSelectedPromptContent] = useState("");

  // Fetch all settings as flat KV map
  const settingsQuery = useQuery({
    queryKey: ["settings"],
    queryFn: () => settingsApi.list(),
  });

  const promptsQuery = useQuery({
    queryKey: ["prompt-admin"],
    queryFn: () => api.get<PromptListView>("/api/v1/prompts"),
  });

  const contactsQuery = useQuery({
    queryKey: ["contacts"],
    queryFn: () => api.get<TelegramContact[]>("/api/v1/contacts"),
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

  // Prompt selection
  useEffect(() => {
    const prompts = promptsQuery.data?.prompts ?? [];
    if (prompts.length === 0) return;
    if (!selectedPromptName || !prompts.some((p) => p.name === selectedPromptName)) {
      const preferred = prompts.find((p) => p.name === "agent/soul.md");
      const initial = preferred ?? prompts[0];
      setSelectedPromptName(initial.name);
      setSelectedPromptContent(initial.content);
      return;
    }
    const matched = prompts.find((p) => p.name === selectedPromptName);
    if (matched) {
      setSelectedPromptContent(matched.content);
    }
  }, [promptsQuery.data, selectedPromptName]);

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

  // Contact mutations
  const createContactMutation = useMutation({
    mutationFn: (req: CreateContactRequest) =>
      api.post<TelegramContact>("/api/v1/contacts", req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
      setContactDialogOpen(false);
      setToast({ kind: "success", message: "Contact created." });
    },
    onError: (e: unknown) => {
      setToast({ kind: "error", message: e instanceof Error ? e.message : "Failed to create contact" });
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
      setToast({ kind: "error", message: e instanceof Error ? e.message : "Failed to update contact" });
    },
  });

  const deleteContactMutation = useMutation({
    mutationFn: (id: string) => api.del(`/api/v1/contacts/${id}`),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["contacts"] });
      setToast({ kind: "success", message: "Contact deleted." });
    },
    onError: (e: unknown) => {
      setToast({ kind: "error", message: e instanceof Error ? e.message : "Failed to delete contact" });
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
  const availablePrompts = promptsQuery.data?.prompts ?? [];
  const selectedPromptMeta = availablePrompts.find((p) => p.name === selectedPromptName);

  const settingsNavItems: Array<{
    id: SettingsPage;
    label: string;
    icon: ReactNode;
    summary: string;
  }> = [
    { id: "general", label: "General", icon: <Palette className="h-4 w-4" />, summary: "Appearance and documentation" },
    { id: "providers", label: "Providers", icon: <Sparkles className="h-4 w-4" />, summary: "LLM provider and model config" },
    { id: "prompts", label: "Prompts", icon: <Sparkles className="h-4 w-4" />, summary: "Prompt admin and templates" },
    { id: "skills", label: "Skills", icon: <Bot className="h-4 w-4" />, summary: "Installed skills and management" },
    { id: "mcp", label: "MCP Servers", icon: <ExternalLink className="h-4 w-4" />, summary: "Tool server connections" },
    { id: "channels", label: "Channels", icon: <MessageSquare className="h-4 w-4" />, summary: "Telegram, Gmail, contacts" },
    { id: "tools", label: "Tools", icon: <Settings2 className="h-4 w-4" />, summary: "Composio, memory integrations" },
  ];

  return (
    <div className="grid gap-4 xl:grid-cols-[16rem_minmax(0,1fr)]">
      {/* Sidebar */}
      <aside className="data-panel h-fit xl:sticky xl:top-4">
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
      <div className="space-y-6">
        {/* Toast */}
        {toast && (
          <div className={cn(
            "rounded-lg border px-4 py-2 text-sm",
            toast.kind === "success" ? "border-green-200 bg-green-50 text-green-700 dark:border-green-800 dark:bg-green-950 dark:text-green-300" : "border-destructive/30 bg-destructive/5 text-destructive"
          )}>
            {toast.message}
          </div>
        )}

        {/* ── General ── */}
        {activeCategory === "general" && (
          <>
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
                  <a
                    href="/swagger-ui/"
                    target="_blank"
                    rel="noreferrer"
                    className="group rounded-xl border bg-card p-4 transition-colors hover:bg-accent/30"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex items-center gap-3">
                        <ExternalLink className="h-4 w-4 text-muted-foreground" />
                        <div>
                          <p className="font-medium">API Reference</p>
                          <p className="text-xs text-muted-foreground">Swagger UI</p>
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
          <>
            <KvGroup
              title="LLM Provider"
              description="Primary provider and API credentials"
              icon={<Sparkles className="h-4 w-4" />}
              fields={[
                { key: KEYS.LLM_PROVIDER, label: "Provider", placeholder: "openrouter", description: "openrouter, ollama, or codex" },
                { key: KEYS.LLM_OPENROUTER_API_KEY, label: "OpenRouter API Key", placeholder: "sk-or-v1-..." },
                { key: KEYS.LLM_OLLAMA_BASE_URL, label: "Ollama Base URL", placeholder: "http://localhost:11434" },
              ]}
              values={draft}
              original={original}
              onFieldChange={handleFieldChange}
              onSave={() => handleGroupSave([KEYS.LLM_PROVIDER, KEYS.LLM_OPENROUTER_API_KEY, KEYS.LLM_OLLAMA_BASE_URL], "llm-provider")}
              saving={saveMutation.isPending}
              toast={groupToasts["llm-provider"] ?? null}
            />
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
              onFieldChange={handleFieldChange}
              onSave={() => handleGroupSave([KEYS.LLM_MODELS_DEFAULT, KEYS.LLM_MODELS_CHAT, KEYS.LLM_MODELS_JOB, KEYS.LLM_FALLBACK_MODELS, KEYS.LLM_FAVORITE_MODELS], "llm-models")}
              saving={saveMutation.isPending}
              toast={groupToasts["llm-models"] ?? null}
            />
          </>
        )}

        {/* ── Prompts ── */}
        {activeCategory === "prompts" && (
          <Card className="app-surface border-border/60">
            <CardHeader>
              <div className="flex items-center gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
                  <Sparkles className="h-4 w-4" />
                </div>
                <div>
                  <CardTitle className="text-base">Prompts</CardTitle>
                  <CardDescription>{availablePrompts.length} prompt files</CardDescription>
                </div>
              </div>
            </CardHeader>
            <CardContent>
              {promptsQuery.isLoading ? (
                <Skeleton className="h-48 w-full" />
              ) : availablePrompts.length === 0 ? (
                <p className="text-sm text-muted-foreground">No prompts found.</p>
              ) : (
                <div className="grid gap-4 lg:grid-cols-[14rem_1fr]">
                  <div className="space-y-1">
                    {availablePrompts.map((p) => (
                      <button
                        key={p.name}
                        type="button"
                        onClick={() => {
                          setSelectedPromptName(p.name);
                          setSelectedPromptContent(p.content);
                        }}
                        className={cn(
                          "w-full rounded-lg px-3 py-2 text-left text-sm transition-colors",
                          selectedPromptName === p.name
                            ? "bg-primary/10 font-medium text-foreground"
                            : "text-muted-foreground hover:bg-accent/40"
                        )}
                      >
                        <p className="truncate">{p.name}</p>
                        <p className="truncate text-xs text-muted-foreground">{p.description}</p>
                      </button>
                    ))}
                  </div>
                  <div className="space-y-3">
                    {selectedPromptMeta && (
                      <div>
                        <h3 className="font-semibold">{selectedPromptMeta.name}</h3>
                        <p className="text-sm text-muted-foreground">{selectedPromptMeta.description}</p>
                      </div>
                    )}
                    <Textarea
                      value={selectedPromptContent}
                      readOnly
                      className="min-h-[400px] font-mono text-xs"
                    />
                  </div>
                </div>
              )}
            </CardContent>
          </Card>
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

            {/* Contacts */}
            <Card className="app-surface border-border/60">
              <CardHeader>
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-3">
                    <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border bg-muted/40 text-muted-foreground">
                      <Users className="h-4 w-4" />
                    </div>
                    <div>
                      <CardTitle className="text-base">Telegram Contacts</CardTitle>
                      <CardDescription>
                        {contactsQuery.data?.length ?? 0} contacts, {contactsQuery.data?.filter((c) => c.enabled).length ?? 0} enabled
                      </CardDescription>
                    </div>
                  </div>
                  <Button size="sm" variant="outline" onClick={openNewContact}>
                    <Plus className="mr-1.5 h-3.5 w-3.5" />
                    Add
                  </Button>
                </div>
              </CardHeader>
              <CardContent>
                {contactsQuery.isLoading ? (
                  <Skeleton className="h-24 w-full" />
                ) : !contactsQuery.data || contactsQuery.data.length === 0 ? (
                  <p className="text-sm text-muted-foreground">No contacts yet.</p>
                ) : (
                  <div className="divide-y divide-border/60 rounded-lg border">
                    {contactsQuery.data.map((contact) => (
                      <div key={contact.id} className="flex items-center justify-between px-4 py-3">
                        <div className="min-w-0">
                          <div className="flex items-center gap-2">
                            <p className="truncate text-sm font-medium">{contact.name}</p>
                            <Badge variant={contact.enabled ? "default" : "secondary"} className="text-[10px]">
                              {contact.enabled ? "Enabled" : "Disabled"}
                            </Badge>
                          </div>
                          <p className="truncate text-xs text-muted-foreground">
                            @{contact.telegram_username}
                            {contact.chat_id != null && ` · Chat ID: ${contact.chat_id}`}
                          </p>
                        </div>
                        <div className="flex items-center gap-1 shrink-0">
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7"
                            onClick={() => openEditContact(contact)}
                          >
                            <Pencil className="h-3 w-3" />
                          </Button>
                          <Button
                            variant="ghost"
                            size="icon"
                            className="h-7 w-7 text-muted-foreground hover:text-destructive"
                            onClick={() => {
                              if (window.confirm(`Delete contact "${contact.name}"?`)) {
                                deleteContactMutation.mutate(contact.id);
                              }
                            }}
                          >
                            <Trash2 className="h-3 w-3" />
                          </Button>
                        </div>
                      </div>
                    ))}
                  </div>
                )}
              </CardContent>
            </Card>

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
      </div>

      {/* Contact Dialog */}
      <Dialog open={contactDialogOpen} onOpenChange={(open) => !open && setContactDialogOpen(false)}>
        <DialogContent className="sm:max-w-md">
          <DialogHeader>
            <DialogTitle>{editingContact ? "Edit Contact" : "New Contact"}</DialogTitle>
            <DialogDescription>
              {editingContact ? "Update contact details." : "Add a new Telegram contact."}
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4 py-4">
            <div className="space-y-1.5">
              <Label htmlFor="contact-name">Name</Label>
              <Input
                id="contact-name"
                value={contactName}
                onChange={(e) => setContactName(e.target.value)}
                placeholder="Contact name"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="contact-username">Telegram Username</Label>
              <Input
                id="contact-username"
                value={contactUsername}
                onChange={(e) => setContactUsername(e.target.value)}
                placeholder="username (without @)"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="contact-notes">Notes</Label>
              <Textarea
                id="contact-notes"
                value={contactNotes}
                onChange={(e) => setContactNotes(e.target.value)}
                placeholder="Optional notes"
                rows={2}
              />
            </div>
            <div className="flex items-center gap-2">
              <Switch
                id="contact-enabled"
                checked={contactEnabled}
                onCheckedChange={setContactEnabled}
              />
              <Label htmlFor="contact-enabled">Enabled</Label>
            </div>
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setContactDialogOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={handleSaveContact}
              disabled={!contactName.trim() || !contactUsername.trim() || createContactMutation.isPending || updateContactMutation.isPending}
            >
              {editingContact ? "Update" : "Create"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
